//! NR3 collection-operator differential tests (reference-derived expectations
//! captured from the nift-embed CLI).

use nift::bindings::Bindings;
use nift::context::Context;
use nift::expr::{dump_compact, evaluate_collection_value};
use nift::{InMemoryHost, RenderIdentity, Value};

fn make_host<'a>(defaults: &'a Bindings, context: &'a Context) -> InMemoryHost<'a> {
    InMemoryHost::new(defaults, context, "/site")
}

fn eval_collection(expr: &str) -> String {
    let mut defaults = Bindings::new();
    let mut posts = Value::array();
    for (title, score, published) in [("x", 5.0, true), ("y", 3.0, false), ("z", 7.0, true)] {
        let mut post = Value::object();
        post.insert("title", Value::string(title)).unwrap();
        post.insert("score", Value::number(score)).unwrap();
        post.insert("pub", Value::boolean(published)).unwrap();
        posts.push(post).unwrap();
    }
    defaults.set("posts", posts).unwrap();
    defaults
        .set(
            "nums",
            Value::Array(vec![
                Value::number(3.0),
                Value::number(1.0),
                Value::number(2.0),
            ]),
        )
        .unwrap();
    defaults
        .set(
            "words",
            Value::Array(vec![Value::string("b"), Value::string("a")]),
        )
        .unwrap();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("t").title("T");
    let mut bindings = nift::expr::JsonBindings::new();
    let value = evaluate_collection_value(&mut bindings, &host, &identity, expr, None)
        .expect("collection evaluation should succeed");
    dump_compact(&value)
}

#[test]
fn collection_reference_battery() {
    // Reference-derived expectations from the nift-embed CLI probes.
    assert_eq!(eval_collection("@sort(nums)"), "[\n1,\n2,\n3\n]");
    assert_eq!(eval_collection("@sum(nums)"), "6");
    assert_eq!(eval_collection("@min(nums)"), "1");
    assert_eq!(eval_collection("@max(nums)"), "3");
    assert_eq!(eval_collection("@reverse(nums)"), "[\n2,\n1,\n3\n]");
    assert_eq!(eval_collection("@distinct(words)"), "[\n\"b\",\n\"a\"\n]");
    assert_eq!(
        eval_collection("@map(p : posts => p.title)"),
        "[\n\"x\",\n\"y\",\n\"z\"\n]"
    );
    assert_eq!(
        eval_collection("@filter(p : posts => p.pub && p.score >= 5)"),
        "[\n{\n\"title\": \"x\",\n\"score\": 5,\n\"pub\": true\n},\n{\n\"title\": \"z\",\n\"score\": 7,\n\"pub\": true\n}\n]"
    );
    assert_eq!(
        eval_collection("@reduce(n : nums & acc = 0 => acc + n)"),
        "6"
    );
    assert_eq!(
        eval_collection("@sort(p : posts => p.score desc)"),
        "[\n{\n\"title\": \"z\",\n\"score\": 7,\n\"pub\": true\n},\n{\n\"title\": \"x\",\n\"score\": 5,\n\"pub\": true\n},\n{\n\"title\": \"y\",\n\"score\": 3,\n\"pub\": false\n}\n]"
    );
    assert_eq!(eval_collection("@some(p : posts => p.pub)"), "true");
    assert_eq!(eval_collection("@every(p : posts => p.pub)"), "false");
    assert_eq!(
        eval_collection("@find(p : posts => p.score == 3)"),
        "{\n\"title\": \"y\",\n\"score\": 3,\n\"pub\": false\n}"
    );
}
