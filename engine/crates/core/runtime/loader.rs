use std::{borrow::Cow, cell::RefCell, future::Future, pin::Pin, rc::Rc, sync::Arc};

use deno_core::{
    FsModuleLoader, ModuleLoadOptions, ModuleLoadReferrer, ModuleLoadResponse, ModuleLoader,
    ModuleSource, ModuleSourceCode, ModuleSpecifier, ResolutionKind, SourceCodeCacheInfo,
    error::ModuleLoaderError,
};

use shared::vfs::MountTable;

use super::code_cache::SharedCodeCache;

/// Shared reference to the mount table, updated after `evaluate_module`
/// creates it. The module loader holds a clone and checks it on every
/// resolve/load for sandbox enforcement.
pub(crate) type SharedMountTableRef = Rc<RefCell<Option<Arc<MountTable>>>>;

pub(crate) struct MyModuleLoader {
    inner: FsModuleLoader,
    code_cache: Option<SharedCodeCache>,
    /// Set after evaluate_module creates the mount table.
    mount_table: SharedMountTableRef,
}

impl MyModuleLoader {
    pub fn new(code_cache: Option<SharedCodeCache>, mount_table: SharedMountTableRef) -> Self {
        Self {
            inner: FsModuleLoader,
            code_cache,
            mount_table,
        }
    }
}

impl MyModuleLoader {
    /// Attach code cache info to a loaded module source.
    fn attach_code_cache(cache: &SharedCodeCache, mut source: ModuleSource) -> ModuleSource {
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

    /// Validate that a resolved module URL is within the /code sandbox.
    ///
    /// Returns `Ok(())` if the path is allowed, or an error if it escapes
    /// the mount boundaries.
    fn validate_sandbox(&self, url: &ModuleSpecifier) -> Result<(), ModuleLoaderError> {
        let mt_ref = self.mount_table.borrow();
        let Some(mt) = mt_ref.as_ref() else {
            // Mount table not yet set (before evaluate_module) — allow.
            // In practice, no game modules are loaded before evaluate_module.
            return Ok(());
        };

        let Ok(path) = url.to_file_path() else {
            // Not a file:// URL (e.g., built-in modules) — allow.
            return Ok(());
        };

        if mt.is_allowed_path(&path) {
            return Ok(());
        }

        Err(ModuleLoaderError::generic(format!(
            "Module import blocked: path escapes /code sandbox: {}",
            path.display()
        )))
    }
}

impl MyModuleLoader {
    fn synthetic_pack_path(
        code_dir: &std::path::Path,
        generation: u64,
        relative: &std::path::Path,
    ) -> std::path::PathBuf {
        code_dir
            .join(".pack_gen")
            .join(generation.to_string())
            .join(relative)
    }

    /// Try loading a module from the mount table (pack backend support).
    ///
    /// Returns `Some(ModuleSource)` if the file was read from a pack-backed
    /// mount (where `real_path()` returns `None`).  Returns `None` if the
    /// path is filesystem-backed or the mount table isn't available, allowing
    /// fallback to `FsModuleLoader`.
    fn try_load_from_mount(
        &self,
        url: &ModuleSpecifier,
    ) -> Option<Result<ModuleSource, ModuleLoaderError>> {
        let mt_ref = self.mount_table.borrow();
        let mt = mt_ref.as_ref()?;

        let file_path = url.to_file_path().ok()?;
        // Use code_dir() (always available) instead of base_dir() (None
        // when base is pack-backed).
        let code_dir = mt.code_dir();
        let relative = if let Ok(synthetic_rel) = file_path.strip_prefix(code_dir.join(".pack_gen"))
        {
            let mut comps = synthetic_rel.components();
            let _generation_dir = comps.next()?;
            let mut path = std::path::PathBuf::new();
            for comp in comps {
                path.push(comp.as_os_str());
            }
            path
        } else {
            file_path.strip_prefix(&code_dir).ok()?.to_path_buf()
        };
        let relative_str = relative.to_str()?;

        // Check if this path resolves through a pack backend.
        let resolved = match mt.resolve(relative_str) {
            Some(resolved) => resolved,
            None if mt.has_overlay_for(relative_str) => {
                return Some(Err(ModuleLoaderError::generic(format!(
                    "Module import blocked by mounted overlay shadow: {}",
                    relative_str,
                ))));
            }
            None => return None,
        };
        if resolved.real_path.is_some() {
            // Filesystem-backed: let FsModuleLoader handle it.
            return None;
        }

        // Pack-backed: read bytes from mount table.
        let bytes = match mt.read(relative_str) {
            Ok(b) => b,
            Err(e) => {
                return Some(Err(ModuleLoaderError::generic(format!(
                    "Failed to read module from package: {}: {}",
                    relative_str, e
                ))));
            }
        };

        let code = match String::from_utf8(bytes) {
            Ok(code) => code,
            Err(e) => {
                return Some(Err(ModuleLoaderError::generic(format!(
                    "Module source is not valid UTF-8: {}",
                    e,
                ))));
            }
        };
        let source = ModuleSource::new(
            deno_core::ModuleType::JavaScript,
            ModuleSourceCode::String(code.into()),
            url,
            None,
        );

        Some(Ok(source))
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
        let mut url = self.inner.resolve(spec.as_ref(), referrer, kind)?;

        if let Some(mt) = self.mount_table.borrow().as_ref() {
            if let Ok(file_path) = url.to_file_path() {
                let code_dir = mt.code_dir();
                if let Ok(relative) = file_path.strip_prefix(&code_dir) {
                    if let Some(relative_str) = relative.to_str() {
                        match mt.resolve(relative_str) {
                            Some(resolved) => {
                                if let Some(real_path) = resolved.real_path {
                                    url = ModuleSpecifier::from_file_path(real_path).map_err(
                                        |_| {
                                            ModuleLoaderError::generic(
                                                "failed to remap module path",
                                            )
                                        },
                                    )?;
                                } else {
                                    let synthetic = Self::synthetic_pack_path(
                                        &code_dir,
                                        resolved.mount_generation,
                                        relative,
                                    );
                                    url = ModuleSpecifier::from_file_path(synthetic).map_err(
                                        |_| {
                                            ModuleLoaderError::generic(
                                                "failed to synthesize pack module path",
                                            )
                                        },
                                    )?;
                                }
                            }
                            None if mt.has_overlay_for(relative_str) => {
                                return Err(ModuleLoaderError::generic(format!(
                                    "Module import blocked by mounted overlay shadow: {}",
                                    relative_str,
                                )));
                            }
                            None => {}
                        }
                    }
                }
            }
        }

        // Sandbox enforcement: resolved path must be within mount boundaries.
        self.validate_sandbox(&url)?;

        Ok(url)
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        maybe_referrer: Option<&ModuleLoadReferrer>,
        options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        // Defense in depth: validate sandbox on load too.
        if let Err(e) = self.validate_sandbox(module_specifier) {
            return ModuleLoadResponse::Sync(Err(e));
        }

        // Try pack backend first.  If the module is in a package file,
        // read it directly without touching the filesystem.
        if let Some(result) = self.try_load_from_mount(module_specifier) {
            let cache = self.code_cache.clone();
            return ModuleLoadResponse::Sync(result.and_then(Self::patch_amd).map(|source| {
                match &cache {
                    Some(c) => Self::attach_code_cache(c, source),
                    None => source,
                }
            }));
        }

        // Filesystem fallback (directory-backed mounts).
        let resp = self.inner.load(module_specifier, maybe_referrer, options);
        let cache = self.code_cache.clone();

        match resp {
            ModuleLoadResponse::Sync(result) => ModuleLoadResponse::Sync(
                result.and_then(Self::patch_amd).map(|source| match &cache {
                    Some(c) => Self::attach_code_cache(c, source),
                    None => source,
                }),
            ),

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
