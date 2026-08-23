//! Deterministic property-style tests for the NR1 data model (no external
//! dependencies; a fixed-seed xorshift64 generates the inputs, so failures are
//! reproducible).
//!
//! Invariants exercised over many generated `Value`s / `Context`s:
//! - `clone == original` for every value;
//! - `insert` into an Object materialises a `Null` and round-trips through
//!   `get`;
//! - `insert` rejects non-objects, `push` rejects non-arrays, never panics;
//! - Object member order is insertion order;
//! - precedence: Context overlay wins over Engine default; the default is used
//!   when the Context does not overlay; unknown names resolve to `None`.

use nift::bindings::{resolve, Bindings};
use nift::{Context, Value};

/// Fixed-seed xorshift64.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
    fn bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
    /// Uniform-ish float in a small set of interesting values.
    fn number(&mut self) -> f64 {
        const INTERESTING: [f64; 8] = [0.0, 1.0, -1.0, 0.5, 2.5, 1e6, -1e6, 3.0];
        INTERESTING[self.below(INTERESTING.len())]
    }
    fn string(&mut self) -> String {
        const ALPHABET: &[u8] = b"abcXYZ0123_-";
        let len = 1 + self.below(6);
        (0..len)
            .map(|_| ALPHABET[self.below(ALPHABET.len())] as char)
            .collect()
    }
}

fn random_value(rng: &mut Rng, depth: usize) -> Value {
    if depth == 0 {
        return match rng.below(4) {
            0 => Value::null(),
            1 => Value::boolean(rng.bool()),
            2 => Value::number(rng.number()),
            _ => Value::string(rng.string()),
        };
    }
    match rng.below(6) {
        0 => Value::null(),
        1 => Value::boolean(rng.bool()),
        2 => Value::number(rng.number()),
        3 => Value::string(rng.string()),
        4 => {
            let count = rng.below(4);
            Value::Array((0..count).map(|_| random_value(rng, depth - 1)).collect())
        }
        _ => {
            let mut object = Value::object();
            let count = rng.below(4);
            for _ in 0..count {
                let _ = object.insert(rng.string(), random_value(rng, depth - 1));
            }
            object
        }
    }
}

#[test]
fn property_clone_is_identity() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..10_000 {
        let value = random_value(&mut rng, 4);
        assert_eq!(value.clone(), value);
    }
}

#[test]
fn property_insert_get_roundtrip_and_materialisation() {
    let mut rng = Rng(0x243F_6A88_85A3_08D3);
    for _ in 0..5_000 {
        let value = random_value(&mut rng, 3);
        let mut object = Value::object();
        object.insert("key", value.clone()).unwrap();
        assert_eq!(object.get("key"), Some(&value));

        // Null materialises into an Object on insert.
        let mut null = Value::null();
        null.insert("key", value.clone()).unwrap();
        assert!(null.is_object());
        assert_eq!(null.get("key"), Some(&value));
    }
}

#[test]
fn property_mutation_errors_never_panic() {
    let mut rng = Rng(0xB7E1_5162_8AED_2A6B);
    for _ in 0..5_000 {
        let mut value = random_value(&mut rng, 3);
        match value.insert("k", Value::null()) {
            Ok(()) => assert!(value.is_object()),
            Err(_) => assert!(
                value.is_number() || value.is_string() || value.is_bool() || value.is_array()
            ),
        }
        match value.push(Value::null()) {
            Ok(()) => assert!(value.is_array()),
            Err(_) => assert!(!value.is_array()),
        }
    }
}

#[test]
fn property_object_member_order_is_insertion_order() {
    let mut rng = Rng(0x1319_8A2E_0370_7344);
    for _ in 0..2_000 {
        let count = rng.below(8);
        let mut keys = Vec::new();
        let mut object = Value::object();
        for _ in 0..count {
            let key = rng.string();
            keys.push(key.clone());
            object.insert(key, Value::null()).unwrap();
        }
        let observed: Vec<String> = object.as_object().unwrap().keys().cloned().collect();
        // Insertion order preserved (duplicate keys would collapse, so dedupe).
        let mut expected = Vec::new();
        for key in keys {
            if !expected.contains(&key) {
                expected.push(key);
            }
        }
        assert_eq!(observed, expected);
    }
}

#[test]
fn property_precedence_context_overlay_wins() {
    let mut rng = Rng(0x6A09_E667_F3BC_C908);
    for _ in 0..5_000 {
        let name = format!("v{}", rng.below(10_000));
        let default = random_value(&mut rng, 2);
        let overlay = random_value(&mut rng, 2);

        let mut defaults = Bindings::new();
        defaults.set(name.clone(), default.clone()).unwrap();
        let mut context = Context::new();
        context.set(name.clone(), overlay.clone()).unwrap();

        // Overlay wins.
        assert_eq!(resolve(&defaults, &context, &name), Some(&overlay));
        // Without the overlay, the default is visible.
        assert_eq!(resolve(&defaults, &Context::new(), &name), Some(&default));
        // An unrelated name resolves to None.
        assert_eq!(resolve(&defaults, &context, "unrelated"), None);
    }
}
