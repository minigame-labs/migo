use jni::{
    JNIEnv,
    objects::{JObject, JString},
};

pub(crate) fn get_string_field(
    env: &mut JNIEnv,
    field_name: &str,
    obj: &JObject,
) -> Result<String, String> {
    let val = env
        .get_field(obj, field_name, "Ljava/lang/String;")
        .map_err(|e| format!("Failed to get field '{field_name}': {e}"))?;

    let jobj = val
        .l()
        .map_err(|e| format!("Field '{field_name}' is not an object: {e}"))?;

    let jstr = JString::from(jobj);

    env.get_string(&jstr)
        .map_err(|e| format!("Failed to convert '{field_name}' to Rust String: {e}"))
        .map(|s| s.to_string_lossy().into_owned())
}

pub(crate) fn get_f32(env: &mut JNIEnv, field_name: &str, obj: &JObject) -> Result<f32, String> {
    let val = env
        .get_field(obj, field_name, "F")
        .map_err(|e| format!("Failed to get field '{field_name}': {e}"))?;

    val.f()
        .map_err(|e| format!("Failed to convert field '{field_name}' to f32: {e}"))
}
