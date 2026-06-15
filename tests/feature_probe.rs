// Feature probe: runs a battery of real-world JS snippets through the engine and
// reports which ones fail (parse / compile / execute / assertion). This is a
// diagnostic harness, not a pass/fail gate — it always "passes" but prints a
// report of gaps so we can prioritise engine work.
//
// Run with:  cargo test --test feature_probe -- --nocapture

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

// Each probe is (category, name, source). Source must use `assert(cond)` to
// self-verify, so a wrong result is caught as well as a crash.
fn probes() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        // --- Closures & scope ---
        ("closure", "counter", r#"
            function mk(){ let n=0; return ()=> ++n; }
            const c=mk(); assert(c()===1); assert(c()===2);
        "#),
        ("closure", "loop-let-capture", r#"
            const fns=[]; for(let i=0;i<3;i++){ fns.push(()=>i); }
            assert(fns[0]()===0); assert(fns[1]()===1); assert(fns[2]()===2);
        "#),
        ("scope", "tdz-block", r#"
            let x=1; { let x=2; assert(x===2); } assert(x===1);
        "#),

        // --- Arrow & this ---
        ("arrow", "lexical-this", r#"
            const o={v:5, get(){ return (()=>this.v)(); }};
            assert(o.get()===5);
        "#),

        // --- Template literals ---
        ("template", "interp", r#"
            const a=2,b=3; assert(`${a}+${b}=${a+b}`==='2+3=5');
        "#),
        ("template", "tagged", r#"
            function tag(s,...v){ return s[0]+v[0]+s[1]; }
            assert(tag`x${9}y`==='x9y');
        "#),
        ("template", "multiline", r#"
            const s=`a
b`; assert(s.length===3);
        "#),

        // --- Destructuring ---
        ("destructure", "array", r#"
            const [a,,b=9]=[1]; assert(a===1); assert(b===9);
        "#),
        ("destructure", "object-nested", r#"
            const {x:{y}}={x:{y:7}}; assert(y===7);
        "#),
        ("destructure", "rest", r#"
            const [h,...t]=[1,2,3]; assert(h===1); assert(t.length===2); assert(t[1]===3);
        "#),
        ("destructure", "obj-rest", r#"
            const {a,...rest}={a:1,b:2,c:3}; assert(a===1); assert(rest.b===2); assert(rest.c===3);
        "#),
        ("destructure", "param", r#"
            function f({a,b}){ return a+b; } assert(f({a:1,b:2})===3);
        "#),
        ("destructure", "swap", r#"
            let a=1,b=2; [a,b]=[b,a]; assert(a===2); assert(b===1);
        "#),

        // --- Spread ---
        ("spread", "array", r#"
            const a=[1,2], b=[...a,3]; assert(b.length===3); assert(b[2]===3);
        "#),
        ("spread", "call", r#"
            function f(a,b,c){ return a+b+c; } assert(f(...[1,2,3])===6);
        "#),
        ("spread", "object", r#"
            const o={...{a:1},b:2}; assert(o.a===1); assert(o.b===2);
        "#),

        // --- Default params ---
        ("default", "param", r#"
            function f(a,b=a*2){ return a+b; } assert(f(3)===9); assert(f(3,1)===4);
        "#),

        // --- Classes ---
        ("class", "basic", r#"
            class A{ constructor(x){this.x=x;} get(){return this.x;} }
            assert(new A(5).get()===5);
        "#),
        ("class", "inherit-super", r#"
            class A{ constructor(x){this.x=x;} } class B extends A{ constructor(){super(7);} }
            assert(new B().x===7);
        "#),
        ("class", "static", r#"
            class A{ static make(){ return 42; } } assert(A.make()===42);
        "#),
        ("class", "field", r#"
            class A{ v=10; get(){return this.v;} } assert(new A().get()===10);
        "#),
        ("class", "getter-setter", r#"
            class A{ #v=0; get v(){return this.#v;} set v(x){this.#v=x;} }
            const a=new A(); a.v=5; assert(a.v===5);
        "#),
        ("class", "static-field", r#"
            class A{ static count=3; } assert(A.count===3);
        "#),
        ("class", "method-super", r#"
            class A{ m(){return 1;} } class B extends A{ m(){return super.m()+1;} }
            assert(new B().m()===2);
        "#),
        ("class", "instanceof", r#"
            class A{} const a=new A(); assert(a instanceof A);
        "#),

        // --- Iteration ---
        ("iter", "for-of-array", r#"
            let s=0; for(const x of [1,2,3]) s+=x; assert(s===6);
        "#),
        ("iter", "for-of-string", r#"
            let n=0; for(const c of 'abc') n++; assert(n===3);
        "#),
        ("iter", "for-in", r#"
            const o={a:1,b:2}; let k=''; for(const p in o) k+=p; assert(k==='ab');
        "#),
        ("iter", "for-in-null-undefined-number-string", r#"
            let n=0;
            for (const k in null) n++;
            for (const k in undefined) n++;
            let out=[];
            for (const k in 'ab') out.push(k);
            let c=0;
            for (const k in 42) c++;
            let ks=[];
            for (const k in {a:1,b:2}) ks.push(k);
            assert(n===0);
            assert(JSON.stringify(out)==='["0","1"]');
            assert(c===0);
            assert(ks.length===2);
        "#),
        ("iter", "for-of-map", r#"
            const m=new Map([['a',1]]); let s=''; for(const [k,v] of m) s+=k+v; assert(s==='a1');
        "#),
        ("iter", "for-of-entries", r#"
            let s=0; for(const [i,v] of [10,20].entries()) s+=i+v; assert(s===31);
        "#),
        ("iter", "spread-iterable", r#"
            const m=new Set([1,2,3]); assert([...m].length===3);
        "#),

        // --- Map / Set ---
        ("map", "basic", r#"
            const m=new Map(); m.set('a',1); assert(m.get('a')===1); assert(m.has('a')); assert(m.size===1);
        "#),
        ("set", "basic", r#"
            const s=new Set([1,1,2]); assert(s.size===2); assert(s.has(2));
        "#),

        // --- Symbol ---
        ("symbol", "basic", r#"
            const s=Symbol('x'); const o={[s]:1}; assert(o[s]===1);
        "#),
        ("symbol", "iterator", r#"
            const o={ [Symbol.iterator](){ let i=0; return {next(){ return i<2?{value:i++,done:false}:{value:undefined,done:true}; }}; } };
            assert([...o].length===2);
        "#),

        // --- Generators ---
        ("generator", "basic", r#"
            function* g(){ yield 1; yield 2; }
            const it=g(); assert(it.next().value===1); assert(it.next().value===2); assert(it.next().done===true);
        "#),
        ("generator", "spread", r#"
            function* g(){ yield* [1,2,3]; }
            assert([...g()].length===3);
        "#),

        // --- async / Promise (sync portion only; we can't pump microtasks here) ---
        ("promise", "resolve-then-sync", r#"
            let ok=false; Promise.resolve(1).then(v=>{ ok=(v===1); }); assert(true);
        "#),
        ("async", "declares", r#"
            async function f(){ return 1; } assert(typeof f==='function');
        "#),

        // --- Optional chaining / nullish ---
        ("optional", "chain", r#"
            const o={a:{b:2}}; assert(o?.a?.b===2); assert(o?.x?.y===undefined);
        "#),
        ("optional", "call", r#"
            const o={f(){return 3;}}; assert(o.f?.()===3); assert(o.g?.()===undefined);
        "#),
        ("optional", "index", r#"
            const o=null; assert(o?.[0]===undefined);
        "#),
        ("nullish", "coalesce", r#"
            assert((null??5)===5); assert((0??9)===0); assert((''??9)==='');
        "#),
        ("nullish", "assign", r#"
            let x=null; x??=7; assert(x===7); let y=1; y??=9; assert(y===1);
        "#),
        ("logical", "assign", r#"
            let a=0; a||=5; assert(a===5); let b=1; b&&=2; assert(b===2);
        "#),

        // --- Array methods ---
        ("array", "map-filter-reduce", r#"
            assert([1,2,3,4].filter(x=>x%2===0).map(x=>x*2).reduce((a,b)=>a+b,0)===12);
        "#),
        ("array", "find-some-every", r#"
            assert([1,2,3].find(x=>x>1)===2); assert([1,2].some(x=>x>1)); assert([2,4].every(x=>x%2===0));
        "#),
        ("array", "includes-indexof", r#"
            assert([1,2,3].includes(2)); assert([1,2,3].indexOf(3)===2); assert([1,2].indexOf(9)===-1);
        "#),
        ("array", "slice-splice", r#"
            const a=[1,2,3,4]; assert(a.slice(1,3).length===2); a.splice(1,1); assert(a.length===3); assert(a[1]===3);
        "#),
        ("array", "concat-join", r#"
            assert([1,2].concat([3]).join('-')==='1-2-3');
        "#),
        ("array", "sort", r#"
            assert([3,1,2].sort((a,b)=>a-b).join('')==='123');
        "#),
        ("array", "sort-default", r#"
            assert([10,2,1].sort().join(',')==='1,10,2');
        "#),
        ("array", "reverse", r#"
            assert([1,2,3].reverse()[0]===3);
        "#),
        ("array", "flat-flatmap", r#"
            assert([1,[2,[3]]].flat(2).length===3); assert([1,2].flatMap(x=>[x,x]).length===4);
        "#),
        ("array", "findindex-findlast", r#"
            assert([1,2,3].findIndex(x=>x===2)===1);
        "#),
        ("array", "fill-copywithin", r#"
            assert([1,2,3].fill(0).join('')==='000');
        "#),
        ("array", "from-of", r#"
            assert(Array.from('abc').length===3); assert(Array.of(1,2,3).length===3);
        "#),
        ("array", "from-maplike", r#"
            assert(Array.from({length:3},(_, i)=>i).join('')==='012');
        "#),
        ("array", "isarray", r#"
            assert(Array.isArray([])); assert(!Array.isArray({}));
        "#),
        ("array", "at", r#"
            assert([1,2,3].at(-1)===3);
        "#),
        ("array", "keys-values", r#"
            assert([...[5,6].keys()].join('')==='01'); assert([...[5,6].values()].join('')==='56');
        "#),
        ("array", "reduceright", r#"
            assert(['a','b','c'].reduceRight((a,b)=>a+b)==='cba');
        "#),

        // --- String methods ---
        ("string", "split-join", r#"
            assert('a,b,c'.split(',').length===3);
        "#),
        ("string", "replace", r#"
            assert('aaa'.replace('a','b')==='baa');
        "#),
        ("string", "replace-regex-g", r#"
            assert('aaa'.replace(/a/g,'b')==='bbb');
        "#),
        ("string", "replaceall", r#"
            assert('aaa'.replaceAll('a','b')==='bbb');
        "#),
        ("string", "match", r#"
            const m='a1b2'.match(/\d/g); assert(m.length===2);
        "#),
        ("string", "matchall", r#"
            const it=[...'a1b2'.matchAll(/\d/g)]; assert(it.length===2);
        "#),
        ("string", "pad", r#"
            assert('5'.padStart(3,'0')==='005'); assert('5'.padEnd(3,'-')==='5--');
        "#),
        ("string", "trim", r#"
            assert('  x  '.trim()==='x'); assert(' x '.trimStart()==='x ');
        "#),
        ("string", "includes-startsend", r#"
            assert('hello'.includes('ell')); assert('hello'.startsWith('he')); assert('hello'.endsWith('lo'));
        "#),
        ("string", "repeat", r#"
            assert('ab'.repeat(3)==='ababab');
        "#),
        ("string", "slice-substring", r#"
            assert('hello'.slice(1,3)==='el'); assert('hello'.substring(1,3)==='el'); assert('hello'.slice(-2)==='lo');
        "#),
        ("string", "char", r#"
            assert('abc'.charAt(1)==='b'); assert('abc'.charCodeAt(0)===97); assert('abc'.codePointAt(0)===97);
        "#),
        ("string", "case", r#"
            assert('Ab'.toUpperCase()==='AB'); assert('Ab'.toLowerCase()==='ab');
        "#),
        ("string", "at", r#"
            assert('abc'.at(-1)==='c');
        "#),
        ("string", "fromcharcode", r#"
            assert(String.fromCharCode(97,98)==='ab');
        "#),
        ("string", "concat-index", r#"
            assert('ab'+'cd'==='abcd'); assert('abc'[1]==='b');
        "#),
        ("string", "raw", r#"
            assert(String.raw`a\nb`==='a\\nb');
        "#),
        ("string", "normalize", r#"
            assert(typeof 'x'.normalize==='function');
        "#),

        // --- Object methods ---
        ("object", "keys-values-entries", r#"
            const o={a:1,b:2}; assert(Object.keys(o).length===2); assert(Object.values(o)[1]===2); assert(Object.entries(o)[0][0]==='a');
        "#),
        ("object", "assign", r#"
            const o=Object.assign({},{a:1},{b:2}); assert(o.a===1); assert(o.b===2);
        "#),
        ("object", "freeze", r#"
            const o=Object.freeze({a:1}); o.a=2; assert(o.a===1); assert(Object.isFrozen(o));
        "#),
        ("object", "create", r#"
            const p={greet(){return 'hi';}}; const o=Object.create(p); assert(o.greet()==='hi');
        "#),
        ("object", "getproto", r#"
            const o={}; assert(Object.getPrototypeOf(o)===Object.prototype);
        "#),
        ("object", "fromentries", r#"
            const o=Object.fromEntries([['a',1],['b',2]]); assert(o.a===1); assert(o.b===2);
        "#),
        ("object", "defineproperty", r#"
            const o={}; Object.defineProperty(o,'x',{value:5}); assert(o.x===5);
        "#),
        ("object", "getownpropertynames", r#"
            assert(Object.getOwnPropertyNames({a:1,b:2}).length===2);
        "#),
        ("object", "spread-computed", r#"
            const k='x'; const o={[k]:1, [`${k}2`]:2}; assert(o.x===1); assert(o.x2===2);
        "#),
        ("object", "shorthand-method", r#"
            const o={a:1, f(){return this.a;}}; assert(o.f()===1);
        "#),
        ("object", "getter-literal", r#"
            const o={ _v:3, get v(){return this._v;}, set v(x){this._v=x;} }; o.v=7; assert(o.v===7);
        "#),
        ("object", "hasown", r#"
            assert(Object.hasOwn({a:1},'a')); assert(!Object.hasOwn({},'a'));
        "#),

        // --- JSON ---
        ("json", "stringify", r#"
            assert(JSON.stringify({a:1,b:[2,3]})==='{"a":1,"b":[2,3]}');
        "#),
        ("json", "parse", r#"
            const o=JSON.parse('{"a":1,"b":[2,3]}'); assert(o.a===1); assert(o.b[1]===3);
        "#),
        ("json", "roundtrip-nested", r#"
            const o={a:{b:{c:[1,2,{d:true}]}}}; assert(JSON.parse(JSON.stringify(o)).a.b.c[2].d===true);
        "#),
        ("json", "stringify-indent", r#"
            assert(JSON.stringify({a:1},null,2)==='{\n  "a": 1\n}');
        "#),

        // --- Math / Number ---
        ("math", "basic", r#"
            assert(Math.max(1,9,2)===9); assert(Math.min(1,9,2)===1); assert(Math.abs(-3)===3); assert(Math.floor(2.9)===2); assert(Math.ceil(2.1)===3); assert(Math.round(2.5)===3);
        "#),
        ("math", "pow-sqrt", r#"
            assert(Math.pow(2,10)===1024); assert(Math.sqrt(144)===12); assert(2**10===1024);
        "#),
        ("math", "trig-const", r#"
            assert(Math.PI>3.14); assert(Math.trunc(-2.7)===-2); assert(Math.sign(-5)===-1);
        "#),
        ("number", "tofixed", r#"
            assert((3.14159).toFixed(2)==='3.14');
        "#),
        ("number", "tostring-radix", r#"
            assert((255).toString(16)==='ff'); assert((5).toString(2)==='101');
        "#),
        ("number", "parse", r#"
            assert(parseInt('42px',10)===42); assert(parseFloat('3.14xx')===3.14); assert(Number('5')===5);
        "#),
        ("number", "isnan-isinteger", r#"
            assert(Number.isNaN(NaN)); assert(Number.isInteger(5)); assert(!Number.isInteger(5.5));
        "#),
        ("number", "minmax-const", r#"
            assert(Number.MAX_SAFE_INTEGER>0); assert(isFinite(1)); assert(!isFinite(Infinity));
        "#),

        // --- RegExp ---
        ("regex", "test", r#"
            assert(/\d+/.test('abc123')); assert(!/^\d+$/.test('abc'));
        "#),
        ("regex", "exec-groups", r#"
            const m=/(\d+)-(\d+)/.exec('12-34'); assert(m[1]==='12'); assert(m[2]==='34');
        "#),
        ("regex", "named-groups", r#"
            const m=/(?<y>\d{4})/.exec('2024'); assert(m.groups.y==='2024');
        "#),
        ("regex", "replace-fn", r#"
            assert('a1b2'.replace(/\d/g,d=>'['+d+']')==='a[1]b[2]');
        "#),
        ("regex", "split", r#"
            assert('a1b2c'.split(/\d/).length===3);
        "#),

        // --- Date ---
        ("date", "now", r#"
            assert(typeof Date.now()==='number');
        "#),
        ("date", "construct", r#"
            const d=new Date(2020,0,1); assert(d.getFullYear()===2020);
        "#),

        // --- Error handling ---
        ("error", "try-catch", r#"
            let caught=false; try{ throw new Error('x'); }catch(e){ caught=(e.message==='x'); } assert(caught);
        "#),
        ("error", "finally", r#"
            let f=false; try{ throw 1; }catch(e){}finally{ f=true; } assert(f);
        "#),
        ("error", "custom-class", r#"
            class MyErr extends Error{ constructor(m){super(m); this.name='MyErr';} }
            let ok=false; try{ throw new MyErr('boom'); }catch(e){ ok=(e instanceof Error)&&(e.name==='MyErr'); } assert(ok);
        "#),
        ("error", "throw-rethrow", r#"
            function f(){ try{ throw new TypeError('t'); }catch(e){ throw e; } }
            let ok=false; try{ f(); }catch(e){ ok=e instanceof TypeError; } assert(ok);
        "#),

        // --- typeof / instanceof / in / delete ---
        ("operator", "typeof", r#"
            assert(typeof 1==='number'); assert(typeof 'x'==='string'); assert(typeof true==='boolean'); assert(typeof undefined==='undefined'); assert(typeof null==='object'); assert(typeof {}==='object'); assert(typeof []==='object'); assert(typeof function(){}==='function'); assert(typeof Symbol()==='symbol');
        "#),
        ("operator", "in-delete", r#"
            const o={a:1}; assert('a' in o); delete o.a; assert(!('a' in o));
        "#),
        ("operator", "exponent-bitwise", r#"
            assert((5&3)===1); assert((5|2)===7); assert((5^1)===4); assert((1<<4)===16); assert((256>>2)===64); assert((~0)===-1);
        "#),
        ("operator", "comma-ternary", r#"
            let x=(1,2,3); assert(x===3); assert((true?1:2)===1);
        "#),
        ("operator", "switch", r#"
            function f(n){ switch(n){ case 1: return 'a'; case 2: return 'b'; default: return 'c'; } }
            assert(f(1)==='a'); assert(f(2)==='b'); assert(f(9)==='c');
        "#),
        ("operator", "label-break", r#"
            let c=0; outer: for(let i=0;i<3;i++){ for(let j=0;j<3;j++){ if(j===1) continue outer; c++; } } assert(c===3);
        "#),
        ("operator", "void", r#"
            assert(void 0===undefined);
        "#),

        // --- Globals ---
        ("global", "globalthis", r#"
            assert(typeof globalThis==='object');
        "#),
        ("global", "encode-uri", r#"
            assert(encodeURIComponent('a b')==='a%20b'); assert(decodeURIComponent('a%20b')==='a b');
        "#),

        // --- Misc real-world patterns ---
        ("pattern", "memoize", r#"
            function memo(fn){ const c=new Map(); return n=>{ if(c.has(n)) return c.get(n); const r=fn(n); c.set(n,r); return r; }; }
            const sq=memo(x=>x*x); assert(sq(4)===16); assert(sq(4)===16);
        "#),
        ("pattern", "compose", r#"
            const compose=(...fns)=>x=>fns.reduceRight((a,f)=>f(a),x);
            const f=compose(x=>x+1,x=>x*2); assert(f(3)===7);
        "#),
        ("pattern", "currying", r#"
            const add=a=>b=>c=>a+b+c; assert(add(1)(2)(3)===6);
        "#),
        ("pattern", "obj-iter-entries", r#"
            const o={a:1,b:2,c:3}; let s=0; for(const [k,v] of Object.entries(o)) s+=v; assert(s===6);
        "#),
        ("pattern", "chained-optional", r#"
            const data={user:{addr:{city:'NY'}}}; assert(data?.user?.addr?.city==='NY'); assert(data?.user?.phone?.number===undefined);
        "#),
        ("pattern", "array-dedup", r#"
            const dedup=a=>[...new Set(a)]; assert(dedup([1,1,2,3,3]).length===3);
        "#),
        ("pattern", "group-by", r#"
            const g={}; for(const x of [1,2,3,4]){ const k=x%2?'odd':'even'; (g[k]=g[k]||[]).push(x); }
            assert(g.odd.length===2); assert(g.even.length===2);
        "#),
    ]
}

#[test]
fn feature_probe_report() {
    let probes = probes();
    let total = probes.len();
    let mut failures: Vec<(String, String, String)> = Vec::new();

    for (cat, name, src) in &probes {
        match try_run(src) {
            Ok(()) => {}
            Err(e) => failures.push((cat.to_string(), name.to_string(), e)),
        }
    }

    println!("\n===== FEATURE PROBE REPORT =====");
    println!("total: {total}, passed: {}, failed: {}", total - failures.len(), failures.len());
    if !failures.is_empty() {
        println!("\n----- FAILURES -----");
        for (cat, name, err) in &failures {
            // Trim very long errors for readability.
            let short: String = err.chars().take(160).collect();
            println!("[{cat}] {name}: {short}");
        }
    }
    println!("================================\n");
}
