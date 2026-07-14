const HOST_RUNTIME: &str = include_str!("host_runtime.rs");

fn evaluate_module_source() -> &'static str {
    let start = HOST_RUNTIME
        .find("    pub async fn evaluate_module")
        .expect("HostJsRuntime::evaluate_module must exist");
    let tail = &HOST_RUNTIME[start..];
    let end = tail[1..]
        .find("\n    pub ")
        .map(|offset| offset + 1)
        .unwrap_or(tail.len());
    &tail[..end]
}

#[test]
fn signed_launch_verifies_before_loading_untrusted_module() {
    let source = evaluate_module_source();
    let receipt = source
        .find("verify_launch_receipt")
        .expect("launch must attempt the persistent receipt fast path");
    let singleflight = source[receipt..]
        .find(".run_package_verification(")
        .map(|offset| receipt + offset)
        .expect("package verification must await the keyed bounded scheduler path");
    let promotion = source[singleflight..]
        .find("verify_and_promote_for_launch")
        .map(|offset| singleflight + offset)
        .expect("scheduler job must perform exact verification and promotion");
    let module_load = source
        .find("load_main_es_module")
        .expect("evaluate_module must load the selected module");

    assert!(receipt < singleflight);
    assert!(singleflight < promotion);
    assert!(promotion < module_load);
    assert!(!source.contains(".verify_all_files("));
    assert!(!source.contains(".verify_entry("));
}
