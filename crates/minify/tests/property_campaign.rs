//! Deterministic mutation/property campaign for minify-rs.
//!
//! Ports the Minify++ `fuzz_smoke` mutation philosophy into an idiomatic Rust
//! property runner using a fixed xorshift seed (deterministic across runs):
//! representative valid seeds per format are mutated (insert / erase /
//! replace / duplicate of the byte set), then every generated input is checked
//! for:
//!   - no panic on any input;
//!   - deterministic output;
//!   - second-pass acceptance and idempotence for every successful
//!     minification (minify(minify(x)) == minify(x)).
//!
//! Deterministic: the same seed always produces the same case sequence, so
//! failures are reproducible.

use minify::{minify, Format};

/// xorshift64* (matches the reference fuzz harness's PRNG family).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

const BYTES: &[u8] = b" <>/{}[]()'\"`\\;:,.+-*$@!?=\n\r\t09azAZ&%#_";

/// Deterministic mutation of a seed (port of `fuzz_smoke.cpp::mutate`).
fn mutate(value: &str, rng: &mut Rng) -> String {
    let mut value = value.to_string();
    let edits = 1 + (rng.next() % 8) as usize;
    for _ in 0..edits {
        let operation = (rng.next() % 4) as usize;
        let position = if value.is_empty() {
            0
        } else {
            (rng.next() % (value.len() as u64 + 1)) as usize
        };
        if operation == 0 && value.len() < 4096 {
            let b = BYTES[(rng.next() % BYTES.len() as u64) as usize];
            value.insert(position, b as char);
        } else if operation == 1 && !value.is_empty() {
            let pos = if position == value.len() { position - 1 } else { position };
            value.remove(pos);
        } else if operation == 2 && !value.is_empty() {
            let pos = if position == value.len() { position - 1 } else { position };
            let b = BYTES[(rng.next() % BYTES.len() as u64) as usize];
            let mut chars: Vec<char> = value.chars().collect();
            if pos < chars.len() {
                chars[pos] = b as char;
            }
            value = chars.into_iter().collect();
        } else if !value.is_empty() && value.len() < 4096 {
            let start = (rng.next() % value.len() as u64) as usize;
            let length = std::cmp::min(value.len() - start, 1 + (rng.next() % 16) as usize);
            let dup = value[start..start + length].to_string();
            let pos = if position > value.len() { value.len() } else { position };
            value.insert_str(pos, &dup);
        }
    }
    value
}

/// Representative valid seeds per format (the campaign mutates these).
const SEEDS: &[(Format, &str)] = &[
    (Format::Html, "<div class=\"a\"><p>hello world</p><span> x </span></div>"),
    (Format::Css, ".a { color: red; margin: 0 10px; } body { background: url(\"x.png\"); }"),
    (Format::Json, "{\"a\":1,\"b\":[true,null,\"x y\"],\"c\":{\"d\":1.5}}"),
    (Format::Xml, "<root xmlns:x=\"u\"><a x=\"1\">text</a><b><![CDATA[x < y]]></b></root>"),
    (Format::Svg, "<svg viewBox=\"0 0 1 1\"><path d=\"M 0 0 L 1 1\"/></svg>"),
    (
        Format::JavaScript,
        "function f(a,b){return a+b;} const r=/https?:\\/\\//; const t=`x ${a} y`;",
    ),
    (
        Format::Jsx,
        "const el=<div className=\"a\">{ value + 1 }<span>text</span></div>;",
    ),
];

const MUTATIONS_PER_SEED: usize = 4000;

#[test]
fn deterministic_mutation_campaign_no_panic_idempotent() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut generated = 0usize;
    let mut ok_count = 0usize;
    let mut panic_count = 0usize;
    let mut second_pass_rejections = 0usize;
    let mut non_idempotent = 0usize;
    for (index, (format, seed)) in SEEDS.iter().enumerate() {
        let mut local = Rng(0x5DEECE66D ^ (0x9E3779B97F4A7C15u64.wrapping_add(index as u64 * 0x9E37_79B9 + 1)));
        for _ in 0..MUTATIONS_PER_SEED {
            let input = mutate(seed, &mut local);
            generated += 1;
            // No panic is the hard property.
            let first = std::panic::catch_unwind(|| minify(*format, &input));
            match first {
                Err(_) => {
                    panic_count += 1;
                    continue;
                }
                Ok(Ok(out)) => {
                    ok_count += 1;
                    // Determinism: same input twice -> same result.
                    let again = minify(*format, &input).unwrap();
                    assert_eq!(out, again, "nondeterministic output for {format:?}");
                    // SECOND-PASS ACCEPTANCE is a hard property: a successful
                    // first pass whose output the same minifier then REJECTS is
                    // itself a failure. Then require idempotence.
                    match minify(*format, &out) {
                        Err(_) => {
                            second_pass_rejections += 1;
                            if second_pass_rejections <= 5 {
                                eprintln!(
                                    "second-pass rejection [{format:?}]: input={input:?} once={out:?}"
                                );
                            }
                        }
                        Ok(second) => {
                            if second != out {
                                non_idempotent += 1;
                                if non_idempotent <= 5 {
                                    eprintln!(
                                        "non-idempotent [{format:?}]: input={input:?} once={out:?} twice={second:?}"
                                    );
                                }
                            }
                        }
                    }
                }
                Ok(Err(_)) => {}
            }
        }
    }
    println!(
        "property campaign: {generated} generated inputs across {} formats; first-pass OK {ok_count}; \
         panics {panic_count}; second-pass rejections {second_pass_rejections}; non-idempotent {non_idempotent}",
        SEEDS.len()
    );
    assert_eq!(panic_count, 0, "minify-rs must never panic");
    assert_eq!(
        second_pass_rejections, 0,
        "a successful first pass must never be rejected on pass two"
    );
    assert_eq!(non_idempotent, 0, "minify(minify(x)) must equal minify(x)");
    assert!(generated >= 20_000, "campaign must be substantial");
}
