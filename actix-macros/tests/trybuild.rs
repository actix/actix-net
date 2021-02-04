#[test]
fn compile_macros() {
    let t = trybuild::TestCases::new();
    t.pass("tests/trybuild/main-01-basic.rs");
    t.compile_fail("tests/trybuild/main-02-only-async.rs");
    t.pass("tests/trybuild/main-03-fn-params.rs");

    t.pass("tests/trybuild/test-01-basic.rs");
    t.pass("tests/trybuild/test-02-keep-attrs.rs");
    t.compile_fail("tests/trybuild/test-03-only-async.rs");
}
