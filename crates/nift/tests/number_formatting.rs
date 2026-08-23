//! Differential number-formatting corpus (NR2): `$[...]` scalar rendering of
//! `Value::Number(f64)` must match the frozen C++ reference's
//! `std::to_chars` output. Expected strings were captured live from the
//! nift-embed CLI via `@json` probes (values chosen to exercise integer
//! paths, decimal fractions, many-significant-digit values, very small/large
//! magnitudes, exponent boundaries, negatives and -0.0).

use nift::bindings::Bindings;
use nift::context::Context;
use nift::{render, InMemoryHost, RenderIdentity, Source, Value};

fn render_number(value: f64) -> String {
    let mut defaults = Bindings::new();
    defaults.set("n", Value::number(value)).unwrap();
    let context = Context::new();
    let host = InMemoryHost::new(&defaults, &context, "/site");
    let identity = RenderIdentity::new().name("t").title("T");
    render(&host, &identity, &Source::text("$[n]"), None)
        .expect("render should succeed")
        .output
}

#[test]
fn reference_number_battery() {
    // (input, reference output) captured from the frozen C++ reference.
    let cases: &[(f64, &str)] = &[
        (0.5, "0.5"),
        (3.5, "3.5"),
        (0.1, "0.1"),
        (0.12345678901234567, "0.123456789012346"),
        (123.456, "123.456"),
        (0.30000000000000004, "0.3"),
        (1e-7, "1e-07"),
        (1e-5, "1e-05"),
        (1e-4, "0.0001"),
        (1e16, "10000000000000000"),
        (1e14, "100000000000000"),
        (1e20, "1e+20"),
        (-3.5, "-3.5"),
        (-0.0, "0"),
        (123456789012345678.0, "123456789012345680"),
        (100.0, "100"),
        (2.5e-8, "2.5e-08"),
        (1e15, "1000000000000000"),
        (0.00001, "1e-05"),
        (0.2, "0.2"),
        (-1e20, "-1e+20"),
        (f64::MAX, "1.79769313486232e+308"),
        (
            f64::from_bits(0x0000_0000_0000_0001),
            "4.94065645841247e-324",
        ),
        (1.5, "1.5"),
        (0.3333333333333333, "0.333333333333333"),
    ];
    for (input, expected) in cases {
        assert_eq!(&render_number(*input), expected, "number {input:?}");
    }
}

#[test]
fn integer_valued_doubles_render_as_integers() {
    for value in [0.0, 1.0, -1.0, 100.0, 123456789012345678.0, -0.0] {
        let rendered = render_number(value);
        assert!(
            !rendered.contains('.'),
            "{value:?} rendered as {rendered:?}"
        );
        assert!(
            !rendered.contains('e'),
            "{value:?} rendered as {rendered:?}"
        );
    }
}
