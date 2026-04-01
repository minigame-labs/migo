use std::{borrow::Cow, future::Future, pin::Pin};

use deno_core::{
    FsModuleLoader, ModuleLoadOptions, ModuleLoadReferrer, ModuleLoadResponse, ModuleLoader,
    ModuleSource, ModuleSourceCode, ModuleSpecifier, ResolutionKind, SourceCodeCacheInfo,
    error::ModuleLoaderError,
};

use super::code_cache::SharedCodeCache;

pub(crate) struct MyModuleLoader {
    inner: FsModuleLoader,
    code_cache: Option<SharedCodeCache>,
}

impl MyModuleLoader {
    pub fn new(code_cache: Option<SharedCodeCache>) -> Self {
        Self {
            inner: FsModuleLoader,
            code_cache,
        }
    }
}

impl MyModuleLoader {
    /// Attach code cache info to a loaded module source.
    fn attach_code_cache(
        cache: &SharedCodeCache,
        mut source: ModuleSource,
    ) -> ModuleSource {
        let hash = cache.compute_hash(source.code.as_bytes());
        let data = cache.get(hash).map(Cow::Owned);
        source.code_cache = Some(SourceCodeCacheInfo { hash, data });
        source
    }

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
        let code = String::from_utf8_lossy(source.code.as_bytes());

        if code.contains("define.amd") || code.contains("typeof define") {
            let mut patched = code.into_owned();
            patched.push_str("\nexport default globalThis._lastDefinedModule;\n");
            source.code = ModuleSourceCode::String(patched.into());
        } else if shared::cjs_compat::is_cjs(&code) {
            let patched = shared::cjs_compat::wrap_cjs(&code);
            source.code = ModuleSourceCode::String(patched.into());
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
        self.inner.resolve(spec.as_ref(), referrer, kind)
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        maybe_referrer: Option<&ModuleLoadReferrer>,
        options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        let resp = self.inner.load(module_specifier, maybe_referrer, options);
        let cache = self.code_cache.clone();

        match resp {
            ModuleLoadResponse::Sync(result) => {
                ModuleLoadResponse::Sync(result.and_then(Self::patch_amd).map(|source| {
                    match &cache {
                        Some(c) => Self::attach_code_cache(c, source),
                        None => source,
                    }
                }))
            }

            ModuleLoadResponse::Async(fut) => {
                let fut = async move {
                    let source = fut.await?;
                    let source = Self::patch_amd(source)?;
                    Ok(match &cache {
                        Some(c) => Self::attach_code_cache(c, source),
                        None => source,
                    })
                };
                ModuleLoadResponse::Async(Box::pin(fut))
            }
        }
    }

    fn prepare_load(
        &self,
        module_specifier: &ModuleSpecifier,
        maybe_referrer: Option<String>,
        maybe_content: Option<String>,
        options: ModuleLoadOptions,
    ) -> Pin<Box<dyn Future<Output = Result<(), ModuleLoaderError>>>> {
        self.inner
            .prepare_load(module_specifier, maybe_referrer, maybe_content, options)
    }

    fn finish_load(&self) {}

    fn code_cache_ready(
        &self,
        _module_specifier: ModuleSpecifier,
        hash: u64,
        code_cache: &[u8],
    ) -> Pin<Box<dyn Future<Output = ()>>> {
        if let Some(cache) = &self.code_cache {
            cache.set(hash, code_cache);
        }
        Box::pin(std::future::ready(()))
    }

    fn purge_and_prevent_code_cache(&self, _module_specifier: &str) {
        // No per-specifier tracking; stale entries are evicted by hash mismatch
        // or LRU eviction.
    }
}
