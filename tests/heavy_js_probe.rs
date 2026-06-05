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

        // ===== Batch 2: deeper language / stdlib coverage =====

        // --- Array stdlib (immutable update patterns React uses) ---
        ("array2", "slice-concat-spread", r#"
            const a = [1,2,3]; const b = a.slice(0,2).concat([9]);
            assert(b.join(',') === '1,2,9'); assert(a.join(',') === '1,2,3');
            const c = [...a.slice(1), ...[7,8]]; assert(c.join(',') === '2,3,7,8');
        "#),
        ("array2", "splice", r#"
            const a = [1,2,3,4]; const removed = a.splice(1,2,'x');
            assert(a.join(',') === '1,x,4'); assert(removed.join(',') === '2,3');
        "#),
        ("array2", "reduce-no-init", r#"
            assert([1,2,3,4].reduce((a,b)=>a+b) === 10);
            assert([5].reduce((a,b)=>a+b) === 5);
        "#),
        ("array2", "indexOf-lastIndexOf-some-every", r#"
            const a = [1,2,3,2];
            assert(a.indexOf(2) === 1 && a.lastIndexOf(2) === 3);
            assert(a.some(x=>x>2) && !a.every(x=>x>1));
        "#),
        ("array2", "fill-copyWithin-from-set", r#"
            assert([1,2,3].fill(0,1).join(',') === '1,0,0');
            assert(Array.from(new Set([1,1,2,3])).join(',') === '1,2,3');
        "#),
        ("array2", "sort-stable-objects", r#"
            const a = [{k:2,i:0},{k:1,i:1},{k:2,i:2},{k:1,i:3}];
            a.sort((x,y)=>x.k-y.k);
            assert(a.map(o=>o.i).join(',') === '1,3,0,2');
        "#),

        // --- String stdlib ---
        ("string2", "split-slice-trim", r#"
            assert('a,b,c'.split(',').length === 3);
            assert('  hi  '.trim() === 'hi');
            assert('hello'.slice(1,3) === 'el');
            assert('hello'.substring(1,3) === 'el');
        "#),
        ("string2", "replace-fn", r#"
            assert('a1b2'.replace(/(\d)/g, (m,d)=>'['+d+']') === 'a[1]b[2]');
        "#),
        ("string2", "startsWith-endsWith-includes", r#"
            assert('hello'.startsWith('he') && 'hello'.endsWith('lo') && 'hello'.includes('ell'));
        "#),
        ("string2", "template-nested", r#"
            const x = 2; assert(`a${`b${x}c`}d` === 'ab2cd');
        "#),

        // --- Number / Math ---
        ("number2", "toFixed-toString-radix", r#"
            assert((3.14159).toFixed(2) === '3.14');
            assert((255).toString(16) === 'ff');
            assert(parseInt('0xff', 16) === 255);
            assert(parseFloat('3.14abc') === 3.14);
        "#),
        ("number2", "math-spread", r#"
            assert(Math.max(...[3,1,4,1,5]) === 5);
            assert(Math.min(...[3,1,4]) === 1);
            assert(Math.round(2.5) === 3 && Math.floor(2.9) === 2 && Math.ceil(2.1) === 3);
        "#),

        // --- Control flow ---
        ("control", "labeled-break", r#"
            let hits = 0;
            outer: for(let i=0;i<3;i++){ for(let j=0;j<3;j++){ if(j===1) continue outer; hits++; } }
            assert(hits === 3);
        "#),
        ("control", "try-finally-return", r#"
            function f(){ try { return 1; } finally { /* runs but doesn't override */ } }
            assert(f() === 1);
            function g(){ try { throw 'e'; } catch(e){ return 'caught'; } finally {} }
            assert(g() === 'caught');
        "#),
        ("control", "switch-fallthrough", r#"
            function f(x){ let r=''; switch(x){ case 1: r+='a'; case 2: r+='b'; break; default: r+='d'; } return r; }
            assert(f(1) === 'ab' && f(2) === 'b' && f(9) === 'd');
        "#),

        // --- Classes (deeper) ---
        ("class2", "getter-setter-accessor", r#"
            class Temp { #c=0; get celsius(){ return this.#c; } set celsius(v){ this.#c=v; } get f(){ return this.#c*9/5+32; } }
            const t = new Temp(); t.celsius = 100; assert(t.f === 212);
        "#),
        ("class2", "static-private-method", r#"
            class C { static #count=0; static inc(){ return ++C.#count; } #secret(){ return 42; } reveal(){ return this.#secret(); } }
            assert(C.inc() === 1 && C.inc() === 2);
            assert(new C().reveal() === 42);
        "#),
        ("class2", "super-property", r#"
            class A { name(){ return 'A'; } }
            class B extends A { name(){ return super.name() + 'B'; } }
            assert(new B().name() === 'AB');
        "#),
        ("class2", "instanceof-hasInstance", r#"
            class Even { static [Symbol.hasInstance](n){ return n % 2 === 0; } }
            assert(4 instanceof Even); assert(!(3 instanceof Even));
        "#),

        // --- Iteration protocols ---
        ("iter", "generator-delegate", r#"
            function* inner(){ yield 1; yield 2; }
            function* outer(){ yield 0; yield* inner(); yield 3; }
            assert([...outer()].join(',') === '0,1,2,3');
        "#),
        ("iter", "destructure-from-map", r#"
            const m = new Map([['a',1],['b',2]]);
            const out = [];
            for(const [k,v] of m){ out.push(k+v); }
            assert(out.join(',') === 'a1,b2');
        "#),
        ("iter", "spread-set-map-into-array", r#"
            assert([...new Set([1,2,3])].join(',') === '1,2,3');
            assert([...new Map([['x',1]]).values()].join(',') === '1');
        "#),

        // --- Object deeper ---
        ("object2", "enumerable-false-keys", r#"
            const o = {}; Object.defineProperty(o,'hidden',{value:1,enumerable:false});
            o.shown = 2;
            assert(Object.keys(o).join(',') === 'shown');
            assert(o.hidden === 1);
        "#),
        ("object2", "spread-omit-pattern", r#"
            const { a, ...rest } = { a:1, b:2, c:3 };
            assert(a === 1 && rest.b === 2 && rest.c === 3 && rest.a === undefined);
        "#),
        ("object2", "computed-method-shorthand", r#"
            const key = 'run'; const o = { [key](){ return 'ok'; }, val: 1 };
            assert(o.run() === 'ok');
        "#),
        ("object2", "json-stringify-indent", r#"
            assert(JSON.stringify({a:1},null,2) === '{\n  "a": 1\n}');
        "#),

        // --- RegExp ---
        ("regex", "named-groups", r#"
            const m = /(?<y>\d{4})-(?<m>\d{2})/.exec('2024-06');
            assert(m.groups.y === '2024' && m.groups.m === '06');
        "#),
        ("regex", "test-sticky-global", r#"
            assert(/\d+/.test('abc123'));
            const re = /\d/g; let n=0; while(re.exec('a1b2c3')) n++;
            assert(n === 3);
        "#),

        // --- A mini "React-like" component (pure logic, no DOM) ---
        ("framework", "mini-component-render", r#"
            function createElement(type, props, ...children){
                return { type, props: { ...props, children: children.flat() } };
            }
            function renderToString(node){
                if(typeof node === 'string' || typeof node === 'number') return String(node);
                const { type, props } = node;
                const attrs = Object.entries(props).filter(([k])=>k!=='children')
                    .map(([k,v])=>` ${k}="${v}"`).join('');
                const inner = props.children.map(renderToString).join('');
                return `<${type}${attrs}>${inner}</${type}>`;
            }
            function App({name}){ return createElement('div',{class:'app'},
                createElement('h1',null,'Hello ', name), createElement('p',null,'count: ', 3)); }
            const out = renderToString(App({name:'World'}));
            assert(out === '<div class="app"><h1>Hello World</h1><p>count: 3</p></div>');
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
