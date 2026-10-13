//! Bounded MCP reference formatting; diagnostics are a fixed-label allowlist.
use serde_json::{json, Value};
pub fn diagnostics(value: Option<&Value>) -> Option<Value> {
    let value = value?;
    let mode = value["mode"].as_str()?;
    if !matches!(mode, "lexical" | "hybrid") {
        return None;
    }
    let mut status = json!({"mode":mode});
    if mode == "lexical" {
        if let Some(reason) = value["reason"].as_str().filter(|r| {
            matches!(
                *r,
                "semantic_disabled"
                    | "index_not_ready"
                    | "semantic_storage_unavailable"
                    | "embedding_busy"
                    | "embedding_timeout"
                    | "embedding_unavailable"
                    | "embedding_invalid_response"
                    | "embedding_model_changed"
                    | "input_rejected"
            )
        }) {
            status["reason"] = json!(reason);
        }
    }
    Some(status)
}
/// Callers validate a64–2000 token budget; UTF8 bytes provide a conservative cap.
pub fn context(cards: &[Value], budget: usize, retrieval: Option<&Value>) -> Value {
    let mut out = json!({"cards":[],"omitted":false});
    if let Some(status) = diagnostics(retrieval) {
        out["retrieval"] = status;
        if out.to_string().len() > budget {
            out["retrieval"].as_object_mut().unwrap().remove("reason");
            out["omitted"] = json!(true);
        }
    }
    for card in cards {
        let item = json!({"id":card["id"],"title":card["body"]["title"],"revision":card["revision"],"lifecycle":card["body"]["lifecycle"]});
        let mut candidate = out.clone();
        candidate["cards"].as_array_mut().unwrap().push(item);
        if candidate.to_string().len() <= budget {
            out = candidate
        } else {
            out["omitted"] = json!(true)
        }
    }
    out
}
