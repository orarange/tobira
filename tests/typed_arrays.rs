// Regression tests for ArrayBuffer + typed arrays (Uint8Array, Int32Array,
// Float64Array, …): construction, indexed access with per-type coercion,
// shared buffers, statics (from/of), and the core prototype methods.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn uint8_construct_length_and_index() {
    run(r#"
        const a = new Uint8Array(3);
        a[0] = 255;
        assert(a[0] === 255 && a.length === 3);
        assert(a[1] === 0 && a[2] === 0);
    "#);
}

#[test]
fn int32_from_array() {
    run(r#"
        const a = Int32Array.from([1, 2, 3]);
        assert(a.length === 3 && a[2] === 3);
    "#);
}

#[test]
fn construct_from_array_and_iterable() {
    run(r#"
        const a = new Int32Array([10, 20, 30]);
        assert(a.length === 3 && a[0] === 10 && a[2] === 30);
        const b = new Uint8Array(new Set([1, 2, 3]));
        assert(b.length === 3 && b[1] === 2);
    "#);
}

#[test]
fn integer_wrapping_and_clamping() {
    run(r#"
        const u8 = new Uint8Array(1);
        u8[0] = 256; assert(u8[0] === 0);
        u8[0] = -1; assert(u8[0] === 255);
        u8[0] = 300; assert(u8[0] === 44);

        const i8 = new Int8Array(1);
        i8[0] = 200; assert(i8[0] === -56);

        const clamped = new Uint8ClampedArray(1);
        clamped[0] = 300; assert(clamped[0] === 255);
        clamped[0] = -5; assert(clamped[0] === 0);
        clamped[0] = 1.5; assert(clamped[0] === 2);  // round half to even
        clamped[0] = 2.5; assert(clamped[0] === 2);
    "#);
}

#[test]
fn float_views_keep_fraction() {
    run(r#"
        const f = new Float64Array([1.5, -2.25]);
        assert(f[0] === 1.5 && f[1] === -2.25);
        const f32 = new Float32Array(1);
        f32[0] = 0.5;
        assert(f32[0] === 0.5);
    "#);
}

#[test]
fn array_buffer_backed_views() {
    run(r#"
        const buf = new ArrayBuffer(8);
        assert(buf.byteLength === 8);
        const ints = new Int32Array(buf);
        assert(ints.length === 2);
        assert(ints.byteLength === 8);
        assert(ints.byteOffset === 0);
        assert(ints.buffer === buf);
        assert(ints.BYTES_PER_ELEMENT === 4);
    "#);
}

#[test]
fn views_share_the_same_buffer() {
    run(r#"
        const buf = new ArrayBuffer(4);
        const u8 = new Uint8Array(buf);
        const u32 = new Uint32Array(buf);
        u8[0] = 1; u8[1] = 0; u8[2] = 0; u8[3] = 0;
        assert(u32[0] === 1);  // little-endian
        u32[0] = 0x04030201;
        assert(u8[0] === 1 && u8[1] === 2 && u8[2] === 3 && u8[3] === 4);
    "#);
}

#[test]
fn view_with_offset_and_length() {
    run(r#"
        const buf = new ArrayBuffer(8);
        const view = new Uint8Array(buf, 2, 3);
        assert(view.length === 3);
        assert(view.byteOffset === 2);
    "#);
}

#[test]
fn out_of_range_index_is_safe() {
    run(r#"
        const a = new Uint8Array(2);
        a[5] = 99;            // ignored, no throw
        assert(a[5] === undefined);
        assert(a.length === 2);
    "#);
}

#[test]
fn static_of() {
    run(r#"
        const a = Uint16Array.of(1, 2, 3, 4);
        assert(a.length === 4 && a[3] === 4);
    "#);
}

#[test]
fn static_from_with_map_fn() {
    run(r#"
        const a = Int32Array.from([1, 2, 3], x => x * 10);
        assert(a[0] === 10 && a[1] === 20 && a[2] === 30);
    "#);
}

#[test]
fn proto_set_and_subarray() {
    run(r#"
        const a = new Uint8Array(5);
        a.set([1, 2, 3], 1);
        assert(a[0] === 0 && a[1] === 1 && a[2] === 2 && a[3] === 3);
        const sub = a.subarray(1, 4);   // shares the buffer
        assert(sub.length === 3 && sub[0] === 1);
        sub[0] = 9;
        assert(a[1] === 9);             // write reflects back
    "#);
}

#[test]
fn proto_slice_is_a_copy() {
    run(r#"
        const a = new Uint8Array([1, 2, 3, 4]);
        const s = a.slice(1, 3);
        assert(s.length === 2 && s[0] === 2 && s[1] === 3);
        s[0] = 99;
        assert(a[1] === 2);            // original unchanged
    "#);
}

#[test]
fn proto_fill_join_index() {
    run(r#"
        const a = new Uint8Array(4);
        a.fill(7, 1, 3);
        assert(a[0] === 0 && a[1] === 7 && a[2] === 7 && a[3] === 0);
        const b = new Int32Array([5, 6, 7]);
        assert(b.join('-') === '5-6-7');
        assert(b.indexOf(6) === 1);
        assert(b.indexOf(99) === -1);
        assert(b.includes(7) === true);
        assert(b.includes(99) === false);
    "#);
}

#[test]
fn proto_foreach_map_reduce_reverse() {
    run(r#"
        const a = new Int32Array([1, 2, 3]);
        let sum = 0;
        a.forEach(x => { sum += x; });
        assert(sum === 6);

        const doubled = a.map(x => x * 2);
        assert(doubled.length === 3 && doubled[2] === 6);
        assert(doubled instanceof Int32Array);

        assert(a.reduce((acc, x) => acc + x, 0) === 6);
        assert(a.reduce((acc, x) => acc + x) === 6);

        a.reverse();
        assert(a[0] === 3 && a[1] === 2 && a[2] === 1);
    "#);
}

#[test]
fn typed_array_is_iterable() {
    run(r#"
        const a = new Int32Array([4, 5, 6]);
        const out = [];
        for (const x of a) out.push(x);
        assert(out.length === 3 && out[0] === 4 && out[2] === 6);
        const spread = [...a];
        assert(spread.length === 3 && spread[1] === 5);
    "#);
}

#[test]
fn array_buffer_slice_copies_bytes() {
    run(r#"
        const buf = new ArrayBuffer(4);
        const u8 = new Uint8Array(buf);
        u8[0] = 10; u8[1] = 20; u8[2] = 30; u8[3] = 40;
        const copy = buf.slice(1, 3);
        assert(copy.byteLength === 2);
        const view = new Uint8Array(copy);
        assert(view[0] === 20 && view[1] === 30);
    "#);
}
