fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&wk_capture::event_schema()).unwrap()
    );
}
