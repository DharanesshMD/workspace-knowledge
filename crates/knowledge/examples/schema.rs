fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&wk_knowledge::lifecycle::policy_schema()).unwrap()
    );
}
