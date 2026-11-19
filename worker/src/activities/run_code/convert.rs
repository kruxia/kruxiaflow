//! JSON ↔ Monty value conversion.
//!
//! `monty-types` has no `serde_json` support, so `run_code` converts its
//! inputs, capability arguments and results itself. Conversion out of the
//! sandbox is strict: anything without a faithful JSON form is an error whose
//! message is written for the model that wrote the code.

use monty_types::{MontyObject, ObjectRef};
use serde_json::{Map, Number, Value};

/// Deepest nesting accepted when converting a sandbox value to JSON
const MAX_DEPTH: usize = 64;

/// Largest number of values converted out of one sandbox value. Bounds the
/// work when a value shares sub-objects many times over.
const MAX_NODES: usize = 1_000_000;

/// Convert a JSON value into a Monty value.
///
/// Integers that fit in 64 bits become `int`; larger ones become `float`
/// (JSON numbers beyond i64 are rare and JSON itself does not promise more).
pub fn json_to_monty(value: &Value) -> MontyObject {
    match value {
        Value::Null => MontyObject::none(),
        Value::Bool(b) => MontyObject::bool(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => MontyObject::int(i),
            None => MontyObject::float(n.as_f64().unwrap_or(f64::NAN)),
        },
        Value::String(s) => MontyObject::string(s.as_str()),
        Value::Array(items) => MontyObject::list(items.iter().map(json_to_monty)),
        Value::Object(map) => MontyObject::dict(
            map.iter()
                .map(|(k, v)| (MontyObject::string(k.as_str()), json_to_monty(v))),
        ),
    }
}

/// Convert a sandbox value into JSON.
///
/// Accepts `None`, `bool`, `int` (64-bit), `float` (finite), `str`, `list`,
/// `tuple`, `namedtuple` (as arrays), `dict` with `str` or `int` keys, and
/// `Decimal` (as its exact string form, so money keeps its precision).
/// Anything else is an `Err` with a model-facing message.
pub fn monty_to_json(value: ObjectRef<'_>) -> Result<Value, String> {
    let mut budget = MAX_NODES;
    convert(value, 0, &mut budget)
}

fn convert(value: ObjectRef<'_>, depth: usize, budget: &mut usize) -> Result<Value, String> {
    if depth > MAX_DEPTH {
        return Err(format!(
            "the result is nested more than {MAX_DEPTH} levels deep; return a flatter value"
        ));
    }
    if *budget == 0 {
        return Err("the result is too large to return; return a smaller value".to_string());
    }
    *budget -= 1;

    match value.type_name() {
        "NoneType" => Ok(Value::Null),
        "bool" => Ok(Value::Bool(value.as_bool().unwrap_or(false))),
        "int" => value.as_int().map(Value::from).ok_or_else(|| {
            "an integer in the result does not fit in 64 bits; return it as a string \
             (str(n)) instead"
                .to_string()
        }),
        "float" => {
            let f = value.as_float().unwrap_or(f64::NAN);
            Number::from_f64(f).map(Value::Number).ok_or_else(|| {
                format!("the result contains {f}, which JSON cannot represent; return a finite number or None")
            })
        }
        "str" => Ok(Value::String(
            value.as_str().unwrap_or_default().to_string(),
        )),
        "Decimal" => Ok(Value::String(
            value.as_decimal().unwrap_or_default().to_string(),
        )),
        "list" | "tuple" | "namedtuple" => {
            let items = value.items().unwrap_or_default();
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(convert(item, depth + 1, budget)?);
            }
            Ok(Value::Array(out))
        }
        "dict" => {
            let pairs = value.pairs().unwrap_or_default();
            let mut out = Map::new();
            for (key, item) in pairs {
                let key = match key.type_name() {
                    "str" => key.as_str().unwrap_or_default().to_string(),
                    "int" => match key.as_int() {
                        Some(i) => i.to_string(),
                        None => return Err("a dict key does not fit in 64 bits".to_string()),
                    },
                    other => {
                        return Err(format!(
                            "a dict in the result has a key of type {other}; JSON object keys \
                             must be str (or int, converted to str)"
                        ));
                    }
                };
                out.insert(key, convert(item, depth + 1, budget)?);
            }
            Ok(Value::Object(out))
        }
        "set" | "frozenset" => Err(format!(
            "the result contains a {}, which JSON cannot represent; return sorted(list(...)) \
             instead",
            value.type_name()
        )),
        "date" | "datetime" | "time" => Err(format!(
            "the result contains a {}; return it as a string with .isoformat()",
            value.type_name()
        )),
        other => Err(format!(
            "the result contains a value of type {other}, which JSON cannot represent; return \
             only None, bool, int, float, str, list, tuple, dict or Decimal"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip(value: Value) -> Value {
        monty_to_json(json_to_monty(&value).as_ref()).expect("round trip")
    }

    #[test]
    fn test_scalars_round_trip() {
        for value in [
            json!(null),
            json!(true),
            json!(false),
            json!(0),
            json!(-42),
            json!(i64::MAX),
            json!(i64::MIN),
            json!(1.5),
            json!(-0.25),
            json!(""),
            json!("héllo ✓"),
        ] {
            assert_eq!(round_trip(value.clone()), value);
        }
    }

    #[test]
    fn test_containers_round_trip() {
        let value = json!({
            "orders": [
                {"id": "A1", "total": 612.5, "tags": ["new", "vip"], "meta": {}},
                {"id": "A2", "total": 80, "tags": [], "meta": {"note": null}}
            ],
            "count": 2
        });
        assert_eq!(round_trip(value.clone()), value);
    }

    #[test]
    fn test_u64_beyond_i64_becomes_float() {
        let big = json!(u64::MAX);
        let monty = json_to_monty(&big);
        assert_eq!(monty.as_ref().type_name(), "float");
        assert_eq!(
            monty_to_json(monty.as_ref()).unwrap(),
            json!(u64::MAX as f64)
        );
    }

    #[test]
    fn test_tuple_and_namedtuple_items_become_arrays() {
        let tuple = MontyObject::tuple([MontyObject::int(1), MontyObject::string("a")]);
        assert_eq!(monty_to_json(tuple.as_ref()).unwrap(), json!([1, "a"]));
    }

    #[test]
    fn test_decimal_keeps_exact_string() {
        let decimal = MontyObject::decimal("3.30");
        assert_eq!(monty_to_json(decimal.as_ref()).unwrap(), json!("3.30"));
    }

    #[test]
    fn test_int_dict_keys_become_strings() {
        let dict = MontyObject::dict([
            (MontyObject::int(1), MontyObject::string("one")),
            (MontyObject::string("two"), MontyObject::int(2)),
        ]);
        assert_eq!(
            monty_to_json(dict.as_ref()).unwrap(),
            json!({"1": "one", "two": 2})
        );
    }

    #[test]
    fn test_non_string_dict_key_is_rejected() {
        let dict = MontyObject::dict([(
            MontyObject::tuple([MontyObject::int(1)]),
            MontyObject::int(1),
        )]);
        let err = monty_to_json(dict.as_ref()).unwrap_err();
        assert!(err.contains("key of type tuple"), "{err}");
    }

    #[test]
    fn test_sets_are_rejected_with_a_hint() {
        for set in [
            MontyObject::set([MontyObject::int(1)]),
            MontyObject::frozenset([MontyObject::int(1)]),
        ] {
            let err = monty_to_json(set.as_ref()).unwrap_err();
            assert!(err.contains("sorted(list(...))"), "{err}");
        }
    }

    #[test]
    fn test_non_finite_floats_are_rejected() {
        for f in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let err = monty_to_json(MontyObject::float(f).as_ref()).unwrap_err();
            assert!(err.contains("JSON cannot represent"), "{err}");
        }
    }

    #[test]
    fn test_other_types_are_rejected_by_name() {
        let err = monty_to_json(MontyObject::bytes(b"raw".to_vec()).as_ref()).unwrap_err();
        assert!(err.contains("type bytes"), "{err}");
        let err = monty_to_json(MontyObject::complex(1.0, 2.0).as_ref()).unwrap_err();
        assert!(err.contains("type complex"), "{err}");
    }

    #[test]
    fn test_error_inside_container_propagates() {
        let list =
            MontyObject::list([MontyObject::int(1), MontyObject::set([MontyObject::int(2)])]);
        assert!(monty_to_json(list.as_ref()).is_err());
    }

    #[test]
    fn test_nesting_depth_is_capped() {
        let mut value = MontyObject::int(1);
        for _ in 0..(MAX_DEPTH + 5) {
            value = MontyObject::list([value]);
        }
        let err = monty_to_json(value.as_ref()).unwrap_err();
        assert!(err.contains("nested more than"), "{err}");
    }

    #[test]
    fn test_nesting_at_the_limit_is_accepted() {
        let mut value = MontyObject::int(1);
        for _ in 0..MAX_DEPTH {
            value = MontyObject::list([value]);
        }
        assert!(monty_to_json(value.as_ref()).is_ok());
    }
}
