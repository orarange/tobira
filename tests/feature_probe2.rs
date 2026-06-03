// Tier-2 feature probe: harder / edge-case real-world JS patterns, to find the
// next round of engine gaps after the tier-1 probe reached 131/131. Diagnostic
// only — always "passes" but prints a report (run with --nocapture).

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn try_run(source: &str) -> Result<(), String> {
    let program = Parser::new(source).parse().map_err(|e| format!("PARSE: {e:?}"))?;
    let chunk = Compiler::new(&program).compile().map_err(|e| format!("COMPILE: {e:?}"))?;
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).map_err(|e| format!("EXEC: {e:?}"))?;
    Ok(())
}

fn probes() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        // --- typeof / scoping edge cases ---
        ("typeof", "undeclared", "assert(typeof notDeclared === 'undefined');"),
        ("scope", "fn-hoisting", "assert(hoisted() === 1); function hoisted(){ return 1; }"),
        ("scope", "var-hoisting", "assert(typeof v === 'undefined'); var v = 3; assert(v === 3);"),

        // --- try/catch edge ---
        ("try", "optional-catch", "let ok=false; try{ throw 1; }catch{ ok=true; } assert(ok);"),
        ("try", "finally-override", "function f(){ try{ return 1; }finally{ return 2; } } assert(f()===2);"),

        // --- operators ---
        ("op", "exp-assign", "let x=2; x**=3; assert(x===8);"),
        ("op", "chained-optional-call", "const o={a:{b:()=>5}}; assert(o?.a?.b?.()===5); assert(o?.x?.y?.()===undefined);"),
        ("op", "comma", "assert((1,2,3)===3);"),
        ("op", "in-array", "assert(0 in [1]); assert(!(5 in [1]));"),

        // --- new.target / arguments ---
        ("fn", "arguments", "function f(){ return arguments.length; } assert(f(1,2,3)===3);"),
        ("fn", "arguments-values", "function f(){ return arguments[0]+arguments[1]; } assert(f(2,3)===5);"),
        ("fn", "length-name", "function f(a,b){} assert(f.length===2); assert(f.name==='f');"),
        ("fn", "new-target", "function F(){ return new.target!==undefined; } assert(new F() instanceof F);"),

        // --- destructuring edge ---
        ("destr", "nested-default", "const {a:{b=5}={}}={}; assert(b===5);"),
        ("destr", "from-set", "const [x,y]=new Set([1,2]); assert(x===1&&y===2);"),
        ("destr", "computed", "const k='x'; const {[k]:v}={x:9}; assert(v===9);"),
        ("destr", "swap-arr", "let a=[1,2,3]; [a[0],a[2]]=[a[2],a[0]]; assert(a[0]===3&&a[2]===1);"),

        // --- Array (newer) ---
        ("array", "at-neg", "assert([1,2,3].at(-2)===2);"),
        ("array", "tosorted", "const a=[3,1,2]; assert(a.toSorted((x,y)=>x-y).join('')==='123'); assert(a[0]===3);"),
        ("array", "toreversed", "assert([1,2,3].toReversed().join('')==='321');"),
        ("array", "with", "assert([1,2,3].with(1,9).join('')==='193');"),
        ("array", "flat-infinity", "assert([1,[2,[3,[4]]]].flat(Infinity).length===4);"),
        ("array", "of-from-args", "function f(){ return Array.from(arguments); } assert(f(1,2,3).length===3);"),
        ("array", "sort-stable-objs", "const a=[{k:2},{k:1}]; a.sort((x,y)=>x.k-y.k); assert(a[0].k===1);"),
        ("array", "fill-ctor", "assert(new Array(3).fill(0).length===3);"),
        ("array", "findlast", "assert([1,2,3,4].findLast(x=>x%2===0)===4);"),
        ("array", "group-by", "if(Object.groupBy){ const g=Object.groupBy([1,2,3,4],x=>x%2?'o':'e'); assert(g.o.length===2); } else { assert(true); }"),

        // --- String (edge) ---
        ("string", "replaceall-regex", "assert('a1b2'.replaceAll(/\\d/g,'#')==='a#b#');"),
        ("string", "matchall-groups", "const m=[...'a1b2'.matchAll(/(?<d>\\d)/g)]; assert(m[0].groups.d==='1');"),
        ("string", "codepoint-astral", "assert('😀'.codePointAt(0)===128512);"),
        ("string", "iterate-astral", "assert([...'a😀b'].length===3);"),
        ("string", "localecompare", "assert('a'.localeCompare('b')<0);"),
        ("string", "well-formed", "assert('abc'.length===3);"),
        ("string", "replace-dollar", "assert('John Smith'.replace(/(\\w+)\\s(\\w+)/,'$2 $1')==='Smith John');"),

        // --- Object (edge) ---
        ("object", "descriptors", "const o={a:1}; const d=Object.getOwnPropertyDescriptors(o); assert(d.a.value===1);"),
        ("object", "defineprops", "const o={}; Object.defineProperties(o,{x:{value:5,enumerable:true}}); assert(o.x===5);"),
        ("object", "assign-getter", "const s={get x(){return 7;}}; const o=Object.assign({},s); assert(o.x===7);"),
        ("object", "spread-getter", "const s={get x(){return 7;}}; const o={...s}; assert(o.x===7);"),
        ("object", "entries-order", "assert(Object.entries({b:1,a:2}).map(e=>e[0]).join('')==='ba');"),
        ("object", "computed-method", "const k='m'; const o={[k](){return 3;}}; assert(o.m()===3);"),

        // --- Map / Set (edge) ---
        ("map", "iteration-order", "const m=new Map(); m.set('b',1); m.set('a',2); assert([...m.keys()].join('')==='ba');"),
        ("map", "chaining", "const m=new Map().set('a',1).set('b',2); assert(m.size===2);"),
        ("set", "from-string", "assert(new Set('aabbc').size===3);"),
        ("weakmap", "basic", "const wm=new WeakMap(); const k={}; wm.set(k,1); assert(wm.get(k)===1);"),

        // --- Number / Math ---
        ("number", "tolocale", "assert(typeof (1234).toLocaleString()==='string');"),
        ("number", "neg-zero", "assert(Object.is(-0,-0)); assert(!Object.is(-0,0));"),
        ("number", "exp-format", "assert((1e21).toString()==='1e+21');"),
        ("number", "small-format", "assert((0.0000001).toString()==='1e-7');"),
        ("math", "extra", "assert(Math.cbrt(27)===3); assert(Math.log2(8)===3); assert(Math.hypot(3,4)===5);"),

        // --- Symbol ---
        ("symbol", "for-keyfor", "const s=Symbol.for('x'); assert(Symbol.for('x')===s); assert(Symbol.keyFor(s)==='x');"),
        ("symbol", "description", "assert(Symbol('d').description==='d');"),
        ("symbol", "to-string-tag", "assert(typeof Symbol.toStringTag==='symbol');"),

        // --- Promise / async (synchronous-observable) ---
        ("promise", "async-returns-promise", "const f=async()=>1; assert(typeof f().then==='function');"),
        ("promise", "then-chain-type", "assert(typeof Promise.resolve(1).then(x=>x).catch===  'function');"),
        ("async", "await-in-order", "let log=[]; async function f(){ log.push(1); await 0; log.push(3); } f(); log.push(2); assert(log[0]===1&&log[1]===2);"),

        // --- Reflect / Proxy ---
        ("reflect", "get-set", "const o={a:1}; assert(Reflect.get(o,'a')===1); Reflect.set(o,'b',2); assert(o.b===2);"),
        ("reflect", "has-ownkeys", "assert(Reflect.has({a:1},'a')); assert(Reflect.ownKeys({a:1,b:2}).length===2);"),
        ("proxy", "get-trap", "const p=new Proxy({},{get:(t,k)=>k==='x'?42:undefined}); assert(p.x===42);"),
        ("proxy", "set-trap", "let calls=0; const p=new Proxy({},{set(t,k,v){calls++;t[k]=v;return true;}}); p.a=1; assert(calls===1&&p.a===1);"),

        // --- BigInt ---
        ("bigint", "literal", "assert(typeof 10n==='bigint'); assert(10n+5n===15n);"),

        // --- TypedArray / ArrayBuffer ---
        ("typed", "uint8", "const a=new Uint8Array(3); a[0]=255; assert(a[0]===255&&a.length===3);"),
        ("typed", "int32", "const a=Int32Array.from([1,2,3]); assert(a.length===3&&a[2]===3);"),

        // --- structuredClone / globals ---
        ("global", "structured-clone", "const o={a:[1,2]}; const c=structuredClone(o); c.a[0]=9; assert(o.a[0]===1);"),

        // --- class (advanced) ---
        ("class", "static-block", "class A{ static x; static { A.x=5; } } assert(A.x===5);"),
        ("class", "private-method", "class A{ #m(){return 7;} call(){return this.#m();} } assert(new A().call()===7);"),
        ("class", "computed-method-name", "const k='go'; class A{ [k](){return 1;} } assert(new A().go()===1);"),
        ("class", "super-method", "class A{ m(){return 1;} } class B extends A{ m(){return super.m()+10;} } assert(new B().m()===11);"),
        ("class", "instanceof-chain", "class A{} class B extends A{} assert(new B() instanceof A);"),
        ("class", "static-inherit", "class A{ static make(){return new this();} } class B extends A{} assert(B.make() instanceof B);"),

        // --- generators (advanced) ---
        ("generator", "object-method", "const o={ *g(){ yield 1; yield 2; } }; assert([...o.g()].length===2);"),
        ("generator", "class-method", "class A{ *g(){ yield 1; } } assert([...new A().g()].length===1);"),
        ("generator", "return-in-forof", "function* g(){ yield 1; yield 2; yield 3; } let s=0; for(const x of g()){ if(x===2)break; s+=x; } assert(s===1);"),

        // --- JSON (advanced) ---
        ("json", "reviver", "const o=JSON.parse('{\"a\":1}',(k,v)=>typeof v==='number'?v*2:v); assert(o.a===2);"),
        ("json", "replacer-fn", "assert(JSON.stringify({a:1,b:2},(k,v)=>k==='b'?undefined:v)==='{\"a\":1}');"),
        ("json", "tojson", "const o={toJSON(){return 'custom';}}; assert(JSON.stringify(o)==='\"custom\"');"),

        // --- iteration protocol ---
        ("iter", "entries-destructure", "const m=new Map([['a',1],['b',2]]); const o={}; for(const [k,v] of m) o[k]=v; assert(o.a===1&&o.b===2);"),
        ("iter", "array-from-iterator", "function* g(){yield 1;yield 2;} assert(Array.from(g()).length===2);"),
        ("iter", "spread-args", "function add(a,b,c){return a+b+c;} const nums=[1,2,3]; assert(add(...nums)===6);"),
    ]
}

#[test]
fn feature_probe2_report() {
    let probes = probes();
    let total = probes.len();
    let mut failures: Vec<(String, String, String)> = Vec::new();
    for (cat, name, src) in &probes {
        eprintln!("[probe2] running {cat}/{name}");
        if let Err(e) = try_run(src) {
            failures.push((cat.to_string(), name.to_string(), e));
        }
    }
    println!("\n===== FEATURE PROBE 2 REPORT =====");
    println!("total: {total}, passed: {}, failed: {}", total - failures.len(), failures.len());
    if !failures.is_empty() {
        println!("\n----- FAILURES -----");
        for (cat, name, err) in &failures {
            let short: String = err.chars().take(150).collect();
            println!("[{cat}] {name}: {short}");
        }
    }
    println!("==================================\n");
}
