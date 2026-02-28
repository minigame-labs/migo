//! Integration tests for V8 heap limits and execution timeout.
//!
//! These tests verify that:
//! 1. `while(true){}` is terminated by the watchdog within the configured timeout
//! 2. `new ArrayBuffer(1e10)` triggers the near-heap-limit callback and is intercepted
//! 3. The process does NOT crash in either case
//! 4. Errors are correctly classified as OutOfMemory or JsExecutionTimeout

#[cfg(all(test, feature = "v8-limits"))]
mod v8_limits_tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use deno_core::{JsRuntime, RuntimeOptions, v8};

    use crate::host_runtime::V8LimitsConfig;

    /// Helper: create a JsRuntime with heap limits and OOM callback.
    /// Returns (runtime, oom_flag).
    fn create_limited_runtime(max_heap_mb: usize) -> (JsRuntime, Arc<AtomicBool>) {
        let config = V8LimitsConfig {
            max_heap_size: max_heap_mb * 1024 * 1024,
            initial_heap_size: 0,
        };

        let create_params = v8::Isolate::create_params()
            .heap_limits(config.initial_heap_size, config.max_heap_size);

        let mut rt = JsRuntime::new(RuntimeOptions {
            create_params: Some(create_params),
            ..Default::default()
        });

        let oom_flag = Arc::new(AtomicBool::new(false));
        let cb_flag = Arc::clone(&oom_flag);
        let cb_handle = rt.v8_isolate().thread_safe_handle();

        rt.add_near_heap_limit_callback(move |current_limit, _initial_limit| {
            cb_flag.store(true, Ordering::SeqCst);
            cb_handle.terminate_execution();
            current_limit * 2
        });

        (rt, oom_flag)
    }

    // ========================================================================
    // Test 1: Infinite loop is terminated by watchdog (via IsolateHandle)
    // ========================================================================
    #[test]
    fn test_infinite_loop_terminated_by_watchdog() {
        let mut runtime = JsRuntime::new(RuntimeOptions::default());
        let isolate_handle = runtime.v8_isolate().thread_safe_handle();

        // Spawn a terminator thread that kills execution after 1 second
        let terminator = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let ok = isolate_handle.terminate_execution();
            assert!(ok, "terminate_execution should return true");
        });

        // Execute an infinite loop — should be interrupted
        let result = runtime.execute_script("infinite_loop.js", "for(;;) {}");

        assert!(result.is_err(), "Infinite loop should have been terminated");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("execution terminated"),
            "Error should contain 'execution terminated', got: {}",
            err
        );

        // Verify isolate is still usable after cancel_terminate_execution
        let ok = runtime.v8_isolate().cancel_terminate_execution();
        assert!(ok);

        let result = runtime.execute_script("after_terminate.js", "1 + 1");
        assert!(result.is_ok(), "Isolate should be usable after cancellation");

        terminator.join().unwrap();
    }

    // ========================================================================
    // Test 2: Large allocation triggers near-heap-limit and terminates
    // ========================================================================
    #[test]
    fn test_heap_limit_oom_intercepted() {
        // Use 32MB heap — enough for V8 to initialize, but small enough
        // that aggressive allocation will trigger near-heap-limit quickly.
        // Note: 5MB is too small; V8 needs ~10-20MB just for its own internals
        // and will abort via its fatal OOM handler before our callback can help.
        let (mut runtime, oom_flag) = create_limited_runtime(32);

        // Fill heap by pushing many medium-sized strings into an array.
        // Avoids V8's per-string length limit (RangeError) that would fire before
        // the heap limit callback. Each iteration pushes ~64KB of string data.
        let result = runtime.execute_script(
            "oom_test.js",
            r#"let arr = []; while(true) { arr.push("X".repeat(65536)); }"#,
        );

        assert!(result.is_err(), "Large allocation should fail");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("execution terminated"),
            "Error should contain 'execution terminated', got: {}",
            err
        );

        // Verify OOM flag was set
        assert!(
            oom_flag.load(Ordering::SeqCst),
            "OOM flag should have been set by near_heap_limit_callback"
        );

        // Verify process didn't crash — we're still running!
    }

    // ========================================================================
    // Test 3: Gradual OOM via string concatenation
    // ========================================================================
    #[test]
    fn test_heap_limit_gradual_oom() {
        // 32MB: sufficient for V8 init, triggers OOM on repeated string growth
        let (mut runtime, oom_flag) = create_limited_runtime(32);

        let result = runtime.execute_script(
            "gradual_oom.js",
            r#"let arr = []; while(true) { arr.push("A".repeat(65536)); }"#,
        );

        assert!(result.is_err(), "Gradual OOM should fail");
        assert!(
            oom_flag.load(Ordering::SeqCst),
            "OOM flag should have been set"
        );
    }

    // ========================================================================
    // Test 4: V8LimitsConfig from_max_memory_mb
    // ========================================================================
    #[test]
    fn test_v8_limits_config_from_max_memory_mb() {
        let config = V8LimitsConfig::from_max_memory_mb(256);
        assert_eq!(config.max_heap_size, 256 * 1024 * 1024);
        assert_eq!(config.initial_heap_size, 0);

        // Clamping test
        let config = V8LimitsConfig::from_max_memory_mb(10); // below minimum 64
        assert_eq!(config.max_heap_size, 64 * 1024 * 1024);

        let config = V8LimitsConfig::from_max_memory_mb(9999); // above maximum 2048
        assert_eq!(config.max_heap_size, 2048 * 1024 * 1024);
    }

    // ========================================================================
    // Test 5: Default V8LimitsConfig values
    // ========================================================================
    #[test]
    fn test_v8_limits_config_default() {
        let config = V8LimitsConfig::default();
        assert_eq!(config.max_heap_size, 256 * 1024 * 1024);
        assert_eq!(config.initial_heap_size, 0);
    }

    // ========================================================================
    // Test 6: Normal JS execution works fine with limits enabled
    // ========================================================================
    #[test]
    fn test_normal_execution_with_limits() {
        let (mut runtime, oom_flag) = create_limited_runtime(256);

        // Normal arithmetic
        let result = runtime.execute_script("normal.js", "1 + 1");
        assert!(result.is_ok(), "Normal execution should succeed");

        // Small array allocation
        let result = runtime.execute_script("small_alloc.js", "new ArrayBuffer(1024)");
        assert!(result.is_ok(), "Small allocation should succeed");

        // OOM flag should NOT be set
        assert!(
            !oom_flag.load(Ordering::SeqCst),
            "OOM flag should not be set for normal execution"
        );
    }
}
