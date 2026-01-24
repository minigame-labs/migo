use std::{borrow::Cow, future::Future, pin::Pin};

use deno_core::{
    FsModuleLoader, ModuleLoadResponse, ModuleLoader, ModuleSource, ModuleSourceCode,
    ModuleSpecifier, RequestedModuleType, ResolutionKind, error::ModuleLoaderError,
};

pub(crate) struct MyModuleLoader(pub(crate) FsModuleLoader);

impl MyModuleLoader {
    #[inline]
    fn normalize_specifier<'a>(&self, specifier: &'a str, kind: &ResolutionKind) -> Cow<'a, str> {
        let mut s: Cow<'a, str> = if *kind != ResolutionKind::MainModule {
            if specifier.starts_with("./")
                || specifier.starts_with("../")
                || specifier.contains(':')
            {
                Cow::Borrowed(specifier)
            } else {
                Cow::Owned(format!("./{specifier}"))
            }
        } else {
            Cow::Borrowed(specifier)
        };

        let (path_part, suffix_part) = match s.find(['?', '#']) {
            Some(i) => (&s.as_ref()[..i], &s.as_ref()[i..]),
            None => (s.as_ref(), ""),
        };

        let has_js_like_ext = path_part.ends_with(".js")
            || path_part.ends_with(".mjs")
            || path_part.ends_with(".cjs");

        if !has_js_like_ext {
            let new_path = format!("{path_part}.js{suffix_part}");
            s = Cow::Owned(new_path);
        }

        s
    }

    #[inline]
    fn patch_amd(mut source: ModuleSource) -> Result<ModuleSource, ModuleLoaderError> {
        let mut code = String::from_utf8_lossy(source.code.as_bytes()).into_owned();

        if code.contains("define.amd") || code.contains("typeof define") {
            code = format!(
                r#"{orig}
export default globalThis._lastDefinedModule;
"#,
                orig = code
            );
            source.code = ModuleSourceCode::String(code.into());
        }

        Ok(source)
    }
}

impl ModuleLoader for MyModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        kind: ResolutionKind,
    ) -> Result<ModuleSpecifier, ModuleLoaderError> {
        let spec = self.normalize_specifier(specifier, &kind);
        self.0.resolve(spec.as_ref(), referrer, kind)
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        maybe_referrer: Option<&ModuleSpecifier>,
        is_dyn_import: bool,
        requested_module_type: RequestedModuleType,
    ) -> ModuleLoadResponse {
        let resp = self.0.load(
            module_specifier,
            maybe_referrer,
            is_dyn_import,
            requested_module_type,
        );

        match resp {
            ModuleLoadResponse::Sync(result) => {
                ModuleLoadResponse::Sync(result.and_then(Self::patch_amd))
            }

            ModuleLoadResponse::Async(fut) => {
                let fut = async move {
                    let source = fut.await?;
                    Self::patch_amd(source)
                };
                ModuleLoadResponse::Async(Box::pin(fut))
            }
        }
    }

    fn prepare_load(
        &self,
        module_specifier: &ModuleSpecifier,
        maybe_referrer: Option<String>,
        is_dyn_import: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), ModuleLoaderError>>>> {
        self.0
            .prepare_load(module_specifier, maybe_referrer, is_dyn_import)
    }

    fn finish_load(&self) {}

    fn code_cache_ready(
        &self,
        module_specifier: ModuleSpecifier,
        hash: u64,
        code_cache: &[u8],
    ) -> Pin<Box<dyn Future<Output = ()>>> {
        self.0.code_cache_ready(module_specifier, hash, code_cache)
    }
}
