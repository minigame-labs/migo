use jni::{
    JNIEnv,
    objects::{JObject, JString},
};

/// Get an optional String field from a Java object.
/// Returns `None` if the field is null, empty, or cannot be read.
pub(crate) fn get_optional_string_field(
    env: &mut JNIEnv,
    field_name: &str,
    obj: &JObject,
) -> Option<String> {
    let val = match env.get_field(obj, field_name, "Ljava/lang/String;") {
        Ok(v) => v,
        Err(_) => {
            let _ = env.exception_clear();
            return None;
        }
    };
    let jobj = val.l().ok()?;
    if jobj.is_null() {
        return None;
    }
    let jstr = JString::from(jobj);
    let s = env
        .get_string(&jstr)
        .ok()
        .map(|s| s.into())?;
    if s.is_empty() { None } else { Some(s) }
}

pub(crate) fn get_string_field(
    env: &mut JNIEnv,
    field_name: &str,
    obj: &JObject,
) -> Result<String, String> {
    let val = match env.get_field(obj, field_name, "Ljava/lang/String;") {
        Ok(v) => v,
        Err(e) => {
            let _ = env.exception_clear();
            return Err(format!("Failed to get field '{field_name}': {e}"));
        }
    };

    let jobj = val
        .l()
        .map_err(|e| format!("Field '{field_name}' is not an object: {e}"))?;

    if jobj.is_null() {
        return Err(format!("Field '{}' is null", field_name));
    }

    let jstr = JString::from(jobj);

    env.get_string(&jstr)
        .map_err(|e| format!("Failed to convert '{field_name}' to Rust String: {e}"))
        .map(|s| s.into())
}

pub(crate) fn get_f32(env: &mut JNIEnv, field_name: &str, obj: &JObject) -> Result<f32, String> {
    let val = match env.get_field(obj, field_name, "F") {
        Ok(v) => v,
        Err(e) => {
            let _ = env.exception_clear();
            return Err(format!("Failed to get field '{field_name}': {e}"));
        }
    };

    val.f()
        .map_err(|e| format!("Failed to convert field '{field_name}' to f32: {e}"))
}

pub(crate) fn get_i32(env: &mut JNIEnv, field_name: &str, obj: &JObject) -> Result<i32, String> {
    let val = match env.get_field(obj, field_name, "I") {
        Ok(v) => v,
        Err(e) => {
            let _ = env.exception_clear();
            return Err(format!("Failed to get field '{field_name}': {e}"));
        }
    };

    val.i()
        .map_err(|e| format!("Failed to convert field '{field_name}' to i32: {e}"))
}

pub(crate) fn get_bool(env: &mut JNIEnv, field_name: &str, obj: &JObject) -> Result<bool, String> {
    let val = match env.get_field(obj, field_name, "Z") {
        Ok(v) => v,
        Err(e) => {
            let _ = env.exception_clear();
            return Err(format!("Failed to get field '{field_name}': {e}"));
        }
    };

    val.z()
        .map_err(|e| format!("Failed to convert field '{field_name}' to bool: {e}"))
}

/// Get the ordinal value of an enum field.
/// This calls the ordinal() method on the enum object.
pub(crate) fn get_enum_ordinal(
    env: &mut JNIEnv,
    field_name: &str,
    field_sig: &str,
    obj: &JObject,
) -> Result<i32, String> {
    let val = match env.get_field(obj, field_name, field_sig) {
        Ok(v) => v,
        Err(e) => {
            let _ = env.exception_clear();
            return Err(format!("Failed to get enum field '{field_name}': {e}"));
        }
    };

    let enum_obj = val
        .l()
        .map_err(|e| format!("Enum field '{field_name}' is not an object: {e}"))?;

    if enum_obj.is_null() {
        return Ok(0); // Default to first enum value
    }

    let ordinal = match env.call_method(&enum_obj, "ordinal", "()I", &[]) {
        Ok(v) => v,
        Err(e) => {
            let _ = env.exception_clear();
            return Err(format!("Failed to call ordinal() on '{field_name}': {e}"));
        }
    };

    ordinal
        .i()
        .map_err(|e| format!("Failed to get ordinal value for '{field_name}': {e}"))
}
