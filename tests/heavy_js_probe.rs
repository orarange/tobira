// Heavy-JS probe: exercises the kinds of patterns modern frameworks (React, etc.)
// and bundlers emit, to surface engine gaps. Diagnostic harness — always
// "passes" but prints a report. Run with:
//   cargo test --test heavy_js_probe -- --nocapture

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn try_run(source: &str) -> Result<(), String> {
    let program = Parser::new(source)
        .parse()
        .map_err(|e| format!("PARSE: {e:?}"))?;
    let chunk = Compiler::new(&program)
        .compile()
        .map_err(|e| format!("COMPILE: {e:?}"))?;
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).map_err(|e| format!("EXEC: {e:?}"))?;
    Ok(())
}

fn probes() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        // --- Object plumbing frameworks rely on ---
        ("object", "defineProperty-getter", r#"
            const o = {}; let calls = 0;
            Object.defineProperty(o, 'x', { get(){ calls++; return 42; }, configurable:true });
            assert(o.x === 42); assert(o.x === 42); assert(calls === 2);
        "#),
        ("object", "defineProperty-setter", r#"
            const o = {}; let stored = 0;
            Object.defineProperty(o, 'x', { set(v){ stored = v*2; }, get(){ return stored; } });
            o.x = 5; assert(o.x === 10);
        "#),
        ("object", "getOwnPropertyDescriptor", r#"
            const o = { x: 1 };
            const d = Object.getOwnPropertyDescriptor(o, 'x');
            assert(d.value === 1 && d.writable === true && d.enumerable === true);
        "#),
        ("object", "defineProperties", r#"
            const o = {};
            Object.defineProperties(o, { a:{value:1,enumerable:true}, b:{value:2,enumerable:true} });
            assert(o.a === 1 && o.b === 2);
        "#),
        ("object", "assign", r#"
            const t = Object.assign({}, {a:1}, {b:2}, {a:3});
            assert(t.a === 3 && t.b === 2);
        "#),
        ("object", "keys-values-entries", r#"
            const o = {a:1,b:2};
            assert(Object.keys(o).join(',') === 'a,b');
            assert(Object.values(o).join(',') === '1,2');
            assert(Object.entries(o).map(e=>e.join(':')).join(',') === 'a:1,b:2');
        "#),
        ("object", "fromEntries", r#"
            const o = Object.fromEntries([['a',1],['b',2]]);
            assert(o.a === 1 && o.b === 2);
        "#),
        ("object", "create-proto", r#"
            const proto = { greet(){ return 'hi'; } };
            const o = Object.create(proto);
            assert(o.greet() === 'hi');
            assert(Object.getPrototypeOf(o) === proto);
        "#),
        ("object", "freeze-isFrozen", r#"
            const o = Object.freeze({a:1});
            assert(Object.isFrozen(o));
            try { o.a = 2; } catch(e) {}
            assert(o.a === 1);
        "#),
        ("object", "getOwnPropertyNames", r#"
            const o = {a:1,b:2};
            assert(Object.getOwnPropertyNames(o).join(',') === 'a,b');
        "#),
        ("object", "spread-merge", r#"
            const a = {x:1}; const b = {...a, y:2};
            assert(b.x === 1 && b.y === 2);
        "#),
        ("object", "computed-keys", r#"
            const k = 'dyn'; const o = { [k]: 1, [`${k}2`]: 2 };
            assert(o.dyn === 1 && o.dyn2 === 2);
        "#),
        ("object", "getter-setter-literal", r#"
            const o = { _v:1, get v(){ return this._v; }, set v(x){ this._v = x; } };
            o.v = 9; assert(o.v === 9);
        "#),

        // --- Symbols / iteration ---
        ("symbol", "for-keyfor", r#"
            const s = Symbol.for('react.element');
            assert(Symbol.keyFor(s) === 'react.element');
            assert(Symbol.for('react.element') === s);
        "#),
        ("symbol", "custom-iterator", r#"
            const obj = { [Symbol.iterator](){ let i=0; return { next(){ return i<3?{value:i++,done:false}:{value:undefined,done:true}; } }; } };
            assert([...obj].join(',') === '0,1,2');
        "#),
        ("symbol", "toPrimitive", r#"
            const o = { [Symbol.toPrimitive](hint){ return hint === 'number' ? 42 : 'str'; } };
            assert(+o === 42); assert(`${o}` === 'str');
        "#),
        ("coerce", "toString-concat", r#"
            const o = { toString(){ return 'X'; } };
            assert('' + o === 'X'); assert(`${o}` === 'X');
        "#),
        ("coerce", "valueOf-plus", r#"
            const o = { valueOf(){ return 7; } };
            assert(+o === 7); assert(o * 2 === 14); assert(o + 1 === 8);
        "#),
        ("coerce", "array-join-default", r#"
            assert('' + [1,2,3] === '1,2,3');
            assert(`${[1,2]}` === '1,2');
        "#),

        // --- Reflect / Proxy (mobx, vue reactivity) ---
        ("reflect", "get-set-has", r#"
            const o = {a:1};
            assert(Reflect.get(o,'a') === 1);
            Reflect.set(o,'b',2); assert(o.b === 2);
            assert(Reflect.has(o,'a') === true);
            assert(Reflect.ownKeys(o).join(',') === 'a,b');
        "#),
        ("proxy", "get-trap", r#"
            const p = new Proxy({}, { get(t,k){ return k === 'x' ? 99 : t[k]; } });
            assert(p.x === 99);
        "#),
        ("proxy", "set-trap", r#"
            const log = []; const p = new Proxy({}, { set(t,k,v){ log.push(k+'='+v); t[k]=v; return true; } });
            p.a = 1; p.b = 2; assert(log.join(',') === 'a=1,b=2');
        "#),
        ("proxy", "has-deleteProperty", r#"
            const p = new Proxy({a:1}, { has(t,k){ return k === 'magic' || k in t; } });
            assert('magic' in p); assert('a' in p);
        "#),

        // --- Array methods bundlers/libs use ---
        ("array", "from-iterable", r#"
            assert(Array.from('abc').join(',') === 'a,b,c');
            assert(Array.from({length:3}, (_,i)=>i*2).join(',') === '0,2,4');
        "#),
        ("array", "flat-flatMap", r#"
            assert([1,[2,[3]]].flat(2).join(',') === '1,2,3');
            assert([1,2].flatMap(x=>[x,x*10]).join(',') === '1,10,2,20');
        "#),
        ("array", "reduce-find-includes", r#"
            assert([1,2,3].reduce((a,b)=>a+b,0) === 6);
            assert([1,2,3].find(x=>x>1) === 2);
            assert([1,2,3].includes(2));
            assert([1,2,3].findIndex(x=>x===3) === 2);
        "#),
        ("array", "destructure-default-rest", r#"
            const [a=10, b, ...rest] = [undefined, 2, 3, 4];
            assert(a === 10 && b === 2 && rest.join(',') === '3,4');
        "#),
        ("array", "sort-comparator", r#"
            assert([3,1,2].sort((a,b)=>a-b).join(',') === '1,2,3');
        "#),
        ("array", "at", r#"
            assert([1,2,3].at(-1) === 3);
        "#),

        // --- Map / Set ---
        ("map", "basic-iter", r#"
            const m = new Map(); m.set('a',1).set('b',2);
            assert(m.size === 2 && m.get('a') === 1);
            assert([...m.keys()].join(',') === 'a,b');
            assert([...m.entries()].map(e=>e.join(':')).join(',') === 'a:1,b:2');
        "#),
        ("set", "basic", r#"
            const s = new Set([1,2,2,3]);
            assert(s.size === 3 && s.has(2));
            assert([...s].join(',') === '1,2,3');
        "#),
        ("weakmap", "basic", r#"
            const wm = new WeakMap(); const k = {};
            wm.set(k, 5); assert(wm.get(k) === 5 && wm.has(k));
        "#),

        // --- Functions ---
        ("function", "bind-call-apply", r#"
            function f(a,b){ return this.base + a + b; }
            const g = f.bind({base:10}, 1);
            assert(g(2) === 13);
            assert(f.call({base:1}, 2, 3) === 6);
            assert(f.apply({base:1}, [2,3]) === 6);
        "#),
        ("function", "name-length", r#"
            function foo(a,b){} assert(foo.name === 'foo'); assert(foo.length === 2);
            const bar = (x)=>x; assert(bar.name === 'bar');
        "#),
        ("function", "default-params", r#"
            function f(a, b = a*2){ return a + b; }
            assert(f(3) === 9); assert(f(3,1) === 4);
        "#),

        // --- Classes (React components, libs) ---
        ("class", "extends-super", r#"
            class A { constructor(x){ this.x = x; } get(){ return this.x; } }
            class B extends A { constructor(x){ super(x*2); } getDouble(){ return super.get(); } }
            const b = new B(5); assert(b.getDouble() === 10);
        "#),
        ("class", "private-fields", r#"
            class C { #v = 0; inc(){ this.#v++; return this.#v; } }
            const c = new C(); assert(c.inc() === 1 && c.inc() === 2);
        "#),
        ("class", "static", r#"
            class C { static create(){ return new C(); } hi(){ return 'hi'; } }
            assert(C.create().hi() === 'hi');
        "#),
        ("class", "instanceof", r#"
            class A {} class B extends A {}
            assert(new B() instanceof A); assert(new B() instanceof B);
        "#),

        // (Async/microtask timing is covered by phase5_async; a synchronous
        //  probe harness can't observe post-microtask state, so it lives there.)

        // --- Modern syntax bundlers emit ---
        ("syntax", "optional-chaining", r#"
            const o = { a: { b: null } };
            assert(o?.a?.b?.c === undefined);
            assert((o?.a?.b ?? 'fallback') === 'fallback');
            assert((o?.missing?.x ?? 7) === 7);
            const fn = o?.missing?.call; assert(fn === undefined);
            assert(o?.a?.['b'] === null);
        "#),
        ("syntax", "nullish-assign", r#"
            let a = null; a ??= 5; assert(a === 5);
            let b = 1; b ||= 9; assert(b === 1);
            let c = 1; c &&= 9; assert(c === 9);
        "#),
        ("syntax", "typeof-undeclared", r#"
            assert(typeof process === 'undefined');
            assert(typeof __SOMETHING_UNDEFINED__ === 'undefined');
        "#),
        ("syntax", "globalThis", r#"
            globalThis.__probe_marker = 123;
            assert(globalThis.__probe_marker === 123);
        "#),
        ("syntax", "catch-no-binding", r#"
            let ok = false; try { throw 1; } catch { ok = true; }
            assert(ok);
        "#),
        ("syntax", "spread-call", r#"
            function sum(...xs){ return xs.reduce((a,b)=>a+b,0); }
            assert(sum(...[1,2,3], 4) === 10);
        "#),

        // --- Strings ---
        ("string", "pad-repeat", r#"
            assert('5'.padStart(3,'0') === '005');
            assert('ab'.repeat(3) === 'ababab');
            assert('a,b,c'.replaceAll(',','-') === 'a-b-c');
        "#),
        ("string", "matchAll", r#"
            const ms = [...'a1b2'.matchAll(/(\w)(\d)/g)];
            assert(ms.length === 2 && ms[0][1] === 'a' && ms[1][2] === '2');
        "#),

        // --- JSON (config, props) ---
        ("json", "roundtrip-nested", r#"
            const o = { a:[1,2,{b:true}], c:'x' };
            assert(JSON.stringify(JSON.parse(JSON.stringify(o))) === JSON.stringify(o));
        "#),
        ("json", "reviver-replacer", r#"
            const s = JSON.stringify({a:1,b:2}, (k,v)=> k==='b'?undefined:v);
            assert(s === '{"a":1}');
        "#),

        // --- Numbers ---
        ("number", "statics", r#"
            assert(Number.isInteger(5) && !Number.isInteger(5.5));
            assert(Number.isNaN(NaN) && !Number.isNaN(1));
            assert(Number.MAX_SAFE_INTEGER === 9007199254740991);
        "#),

        // --- structuredClone ---
        ("global", "structuredClone", r#"
            const o = { a: [1,2], b: { c: 3 } };
            const cl = structuredClone(o);
            cl.b.c = 99; assert(o.b.c === 3 && cl.b.c === 99);
        "#),

        // --- A tiny "framework" core: signals + computed (vue/solid style) ---
        ("framework", "mini-reactive", r#"
            function signal(v){ const subs = new Set(); return {
                get(){ if(current) subs.add(current); return v; },
                set(nv){ v = nv; subs.forEach(f=>f()); }
            }; }
            let current = null;
            function effect(fn){ current = fn; fn(); current = null; }
            const a = signal(1); let doubled = 0;
            effect(()=>{ doubled = a.get() * 2; });
            assert(doubled === 2);
            a.set(5); assert(doubled === 10);
        "#),

        // --- A tiny virtual DOM diff (no real DOM, pure logic) ---
        ("framework", "vdom-create", r#"
            function h(tag, props, ...children){ return { tag, props: props||{}, children: children.flat() }; }
            const tree = h('div', {id:'a'}, h('span', null, 'hi'), h('b', null, 'x'));
            assert(tree.tag === 'div' && tree.props.id === 'a');
            assert(tree.children.length === 2 && tree.children[0].tag === 'span');
            assert(tree.children[0].children[0] === 'hi');
        "#),
    ]
}

#[test]
fn heavy_js_probe_report() {
    let probes = probes();
    let mut failures: Vec<(&str, &str, String)> = Vec::new();
    for (cat, name, src) in &probes {
        if let Err(e) = try_run(src) {
            failures.push((cat, name, e));
        }
    }
    let total = probes.len();
    let passed = total - failures.len();
    println!("\n=== heavy-js probe: {passed}/{total} passed ===");
    if !failures.is_empty() {
        println!("--- FAILURES ({}): ---", failures.len());
        for (cat, name, err) in &failures {
            println!("  [{cat}] {name}: {err}");
        }
    }
    println!();
    // Diagnostic only — never fails the suite.
}
