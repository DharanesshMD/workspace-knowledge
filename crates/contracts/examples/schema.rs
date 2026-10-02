fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&wk_contracts::submission_schema()).unwrap()
    );
}
