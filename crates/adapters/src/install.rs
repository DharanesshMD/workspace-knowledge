//! Non-destructive strict-JSON merging. JSONC is rejected unchanged.
use serde_json::Value;
pub fn merge(existing: &Value, fragment: &Value) -> Result<Value, String> {
    let mut result = existing.clone();
    let obj = result.as_object_mut().ok_or("config must be an object")?;
    if let Some(version) = fragment.get("version") {
        if obj.get("version").is_some_and(|v| v != version) {
            return Err("incompatible hook config version".into());
        }
        obj.insert("version".into(), version.clone());
    }
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or("hooks must be an object")?;
    for (event, entries) in fragment["hooks"]
        .as_object()
        .ok_or("invalid hook fragment")?
    {
        let list = hooks
            .entry(event)
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .ok_or("hook entries must be an array")?;
        for item in entries.as_array().ok_or("invalid hook entries")? {
            if !list.contains(item) {
                list.push(item.clone());
            }
        }
    }
    Ok(result)
}
pub fn remove(existing: &Value, fragment: &Value) -> Result<Value, String> {
    let mut result = existing.clone();
    let Some(hooks) = result.get_mut("hooks").and_then(Value::as_object_mut) else {
        return Ok(result);
    };
    for (event, entries) in fragment["hooks"]
        .as_object()
        .ok_or("invalid ownership receipt")?
    {
        if let Some(list) = hooks.get_mut(event).and_then(Value::as_array_mut) {
            let owned = entries.as_array().ok_or("invalid receipt")?;
            list.retain(|v| !owned.contains(v));
        }
    }
    Ok(result)
}
