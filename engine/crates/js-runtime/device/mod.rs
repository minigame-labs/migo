use deno_core::Extension;

deno_core::extension!(
    host_v8_device,
    deps = [host_v8_base],
    esm = [
        dir "device",
        "01_device_motion.js",
        "02_gyroscope.js",
        "03_orientation.js",
    ]
);

pub fn device_extensions() -> Vec<Extension> {
    vec![host_v8_device::init_ops_and_esm()]
}
