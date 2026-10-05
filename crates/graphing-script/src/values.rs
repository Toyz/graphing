//! Between Rune values and JSON, the only shapes that cross the script
//! boundary: booleans, numbers, text, lists and objects.

use rune::Value as RuneValue;
use rune::runtime::{Object, Vec as RuneVec};

/// How deep lists and objects may nest crossing the boundary.
const MAX_DEPTH: usize = 64;

fn too_deep() -> String {
    format!("it nests more than {MAX_DEPTH} deep")
}

pub(crate) fn from_json(json: &serde_json::Value) -> Result<RuneValue, String> {
    from_json_at(json, 0)
}

fn from_json_at(json: &serde_json::Value, depth: usize) -> Result<RuneValue, String> {
    use serde_json::Value as Json;
    if depth > MAX_DEPTH {
        return Err(too_deep());
    }
    let converted = match json {
        Json::Null => rune::to_value(None::<RuneValue>),
        Json::Bool(b) => rune::to_value(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => rune::to_value(i),
            None => rune::to_value(n.as_f64().unwrap_or(f64::NAN)),
        },
        Json::String(s) => rune::to_value(s.clone()),
        Json::Array(items) => {
            let mut list = RuneVec::new();
            for item in items {
                list.push(from_json_at(item, depth + 1)?).map_err(|e| e.to_string())?;
            }
            rune::to_value(list)
        }
        Json::Object(map) => {
            let mut object = Object::new();
            for (key, item) in map {
                let key = rune::alloc::String::try_from(key.as_str()).map_err(|e| e.to_string())?;
                object.insert(key, from_json_at(item, depth + 1)?).map_err(|e| e.to_string())?;
            }
            rune::to_value(object)
        }
    };
    converted.map_err(|e| e.to_string())
}

pub(crate) fn to_json(value: &RuneValue) -> Result<serde_json::Value, String> {
    to_json_at(value, 0)
}

fn to_json_at(value: &RuneValue, depth: usize) -> Result<serde_json::Value, String> {
    use serde_json::Value as Json;
    if depth > MAX_DEPTH {
        return Err(too_deep());
    }
    if let Ok(b) = rune::from_value::<bool>(value.clone()) {
        return Ok(Json::Bool(b));
    }
    if let Ok(i) = rune::from_value::<i64>(value.clone()) {
        return Ok(Json::from(i));
    }
    if let Ok(f) = rune::from_value::<f64>(value.clone()) {
        return Ok(serde_json::Number::from_f64(f).map_or(Json::Null, Json::Number));
    }
    // Borrowed, not taken, so the script keeps its own value.
    if let Ok(s) = value.borrow_string_ref() {
        return Ok(Json::String(s.to_string()));
    }
    if let Ok(s) = rune::from_value::<String>(value.clone()) {
        return Ok(Json::String(s));
    }
    if let Ok(None) = rune::from_value::<Option<RuneValue>>(value.clone()) {
        return Ok(Json::Null);
    }
    if let Ok(Some(inner)) = rune::from_value::<Option<RuneValue>>(value.clone()) {
        return to_json_at(&inner, depth + 1);
    }
    let items = value.borrow_ref::<RuneVec>().map(|list| list.iter().cloned().collect::<Vec<RuneValue>>());
    if let Ok(items) = items {
        return items.iter().map(|v| to_json_at(v, depth + 1)).collect::<Result<Vec<_>, _>>().map(Json::Array);
    }
    let fields = value.borrow_ref::<Object>().map(|o| o.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<Vec<(String, RuneValue)>>());
    if let Ok(fields) = fields {
        let mut map = serde_json::Map::new();
        for (k, v) in fields {
            let j = to_json_at(&v, depth + 1).map_err(|e| format!("{k}: {e}"))?;
            map.insert(k, j);
        }
        return Ok(Json::Object(map));
    }
    Err(format!("a {} can't leave the script: only true/false, numbers, text, lists and objects", value.type_info()))
}
