//! The panic hook, end to end: a real panic leaves a real log on disk.
//!
//! Its own test binary on purpose. `crash::install` replaces the process-wide
//! panic hook, so running it beside other tests would rewrite their panic
//! behaviour; alone, it affects nothing else.

#[test]
fn a_panic_writes_a_findable_crash_log() {
    let scratch = sourcelight_lib::test_support::Scratch::new("crash-hook");
    let dir = scratch.join("logs");

    sourcelight_lib::crash::install(dir.clone());

    // Panic on a thread and join it: the panic is contained rather than aborting
    // the test, and the hook runs synchronously on that thread, so by the time
    // join returns the log is already written.
    let _ = std::thread::spawn(|| panic!("deliberate test panic in the pipeline")).join();

    let logs: Vec<_> = std::fs::read_dir(&dir)
        .expect("the hook should have created the log directory")
        .filter_map(Result::ok)
        .collect();
    assert_eq!(
        logs.len(),
        1,
        "exactly one crash log should have been written"
    );

    let name = logs[0].file_name().to_string_lossy().into_owned();
    assert!(
        name.starts_with("crash-") && name.ends_with(".log"),
        "the log is timestamped: {name}"
    );

    let text = std::fs::read_to_string(logs[0].path()).unwrap();
    assert!(
        text.contains("deliberate test panic in the pipeline"),
        "the panic message"
    );
    assert!(text.contains("version:"), "the app version");
    assert!(text.contains("backtrace:"), "a backtrace");
    assert!(text.contains("at:"), "the panic location");
}
