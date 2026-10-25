//! The VC++ runtime a plug-in links, as the host process would provide it.
//!
//! A classic plug-in built with Visual C++ 2008 imports `msvcr90.dll` and
//! carries **no manifest of its own**: Winamp's executable declares the
//! `Microsoft.VC90.CRT` assembly and every plug-in loaded into that process
//! inherits it. A process that declares nothing makes the loader look for a
//! plain `msvcr90.dll` (error 126, or, with a loose copy, the C runtime's own
//! error dialog `R6034`, because that runtime refuses to start outside a
//! side-by-side context).
//!
//! This module gives the host the same thing **only while a plug-in loads**:
//! it finds `Microsoft.VC*.CRT` assembly folders next to the plug-in, one level
//! above it (Winamp keeps plug-ins in `Plugins` and the runtime beside
//! `winamp.exe`) or next to the host, builds an activation context that
//! declares them as dependencies, and activates it on the loading thread.
//! A host that finds no such folder changes nothing.
//!
//! No new trust is involved: the runtime comes from the same tree whose DLLs
//! the host already resolves for the approved plug-in.

use std::path::{Path, PathBuf};

/// One side-by-side assembly identity, as its manifest spells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub version: String,
    pub architecture: String,
    pub public_key_token: String,
}

/// The value of attribute `name` inside the first `<assemblyIdentity ...>` tag.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let at = tag.find(&needle)? + needle.len();
    let rest = &tag[at..];
    Some(rest[..rest.find('"')?].to_string())
}

/// Reads the identity an assembly manifest declares for itself.
pub fn parse_identity(manifest: &str) -> Option<Identity> {
    let start = manifest.find("<assemblyIdentity")?;
    let end = start + manifest[start..].find('>')?;
    let tag = &manifest[start..end];
    Some(Identity {
        name: attribute(tag, "name")?,
        version: attribute(tag, "version")?,
        architecture: attribute(tag, "processorArchitecture")?,
        public_key_token: attribute(tag, "publicKeyToken")?,
    })
}

/// A manifest that declares `identities` as dependencies and nothing else.
pub fn dependency_manifest(identities: &[Identity]) -> String {
    let mut text = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
         <assembly xmlns=\"urn:schemas-microsoft-com:asm.v1\" manifestVersion=\"1.0\">\r\n",
    );
    for identity in identities {
        text.push_str(&format!(
            "<dependency><dependentAssembly><assemblyIdentity type=\"win32\" name=\"{}\" \
             version=\"{}\" processorArchitecture=\"{}\" publicKeyToken=\"{}\"/>\
             </dependentAssembly></dependency>\r\n",
            identity.name, identity.version, identity.architecture, identity.public_key_token
        ));
    }
    text.push_str("</assembly>\r\n");
    text
}

/// The first of `dirs` that holds private runtime assemblies, with their
/// identities: the folder to probe and what it provides.
pub fn find_runtimes(dirs: &[&Path]) -> Option<(PathBuf, Vec<Identity>)> {
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut found = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("Microsoft.VC") && name.ends_with(".CRT")) {
                continue;
            }
            let manifest = entry.path().join(format!("{name}.manifest"));
            let Ok(text) = std::fs::read_to_string(&manifest) else {
                continue;
            };
            if let Some(identity) = parse_identity(&text) {
                found.push(identity);
            }
        }
        if !found.is_empty() {
            return Some((dir.to_path_buf(), found));
        }
    }
    None
}

/// An activated activation context; deactivated and released on drop.
#[cfg(windows)]
pub struct RuntimeContext {
    handle: windows::Win32::Foundation::HANDLE,
    cookie: usize,
}

/// Activates the runtime the plug-in in `plugin_dir` needs, on this thread.
///
/// `scratch` is a folder the host owns, where the generated manifest is
/// written. `None` when no runtime folder exists or the context cannot be
/// built (the load then proceeds as before and reports its own error).
#[cfg(windows)]
pub fn activate_for(
    plugin_dir: &Path,
    host_dir: Option<&Path>,
    scratch: &Path,
) -> Option<RuntimeContext> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::ApplicationInstallationAndServicing::{
        ACTCTXW, ActivateActCtx, CreateActCtxW,
    };

    let mut dirs: Vec<&Path> = vec![plugin_dir];
    dirs.extend(plugin_dir.parent());
    dirs.extend(host_dir);
    let (assembly_dir, identities) = find_runtimes(&dirs)?;
    for identity in &identities {
        crate::fault::trace_event(&format!(
            "side-by-side: {} {} from {}",
            identity.name,
            identity.version,
            assembly_dir.display()
        ));
    }
    let manifest_path = scratch.join("rustamp-runtime.manifest");
    std::fs::write(&manifest_path, dependency_manifest(&identities)).ok()?;

    let wide = |path: &Path| -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };
    let source = wide(&manifest_path);
    let directory = wide(&assembly_dir);
    // ACTCTX_FLAG_ASSEMBLY_DIRECTORY_VALID: probe for private assemblies there.
    let context = ACTCTXW {
        cbSize: std::mem::size_of::<ACTCTXW>() as u32,
        dwFlags: 0x0004,
        lpSource: windows::core::PCWSTR(source.as_ptr()),
        wProcessorArchitecture: 0,
        wLangId: 0,
        lpAssemblyDirectory: windows::core::PCWSTR(directory.as_ptr()),
        lpResourceName: windows::core::PCWSTR::null(),
        lpApplicationName: windows::core::PCWSTR::null(),
        hModule: Default::default(),
    };
    // SAFETY: `context` and the wide buffers it points to are alive for the call.
    let handle = match unsafe { CreateActCtxW(&context) } {
        Ok(handle) => handle,
        Err(error) => {
            crate::fault::trace_event(&format!("side-by-side: CreateActCtxW failed: {error}"));
            return None;
        }
    };
    let mut cookie = 0usize;
    // SAFETY: a context handle just created, and a cookie to write.
    if let Err(error) = unsafe { ActivateActCtx(Some(handle), &mut cookie) } {
        crate::fault::trace_event(&format!("side-by-side: ActivateActCtx failed: {error}"));
        // SAFETY: the handle was created above and is not used again.
        unsafe {
            windows::Win32::System::ApplicationInstallationAndServicing::ReleaseActCtx(handle)
        };
        return None;
    }
    Some(RuntimeContext { handle, cookie })
}

#[cfg(windows)]
impl Drop for RuntimeContext {
    fn drop(&mut self) {
        use windows::Win32::System::ApplicationInstallationAndServicing::{
            DeactivateActCtx, ReleaseActCtx,
        };
        // SAFETY: the cookie and the handle are the ones this value activated.
        unsafe {
            let _ = DeactivateActCtx(0, self.cookie);
            ReleaseActCtx(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CRT_MANIFEST: &str = "<?xml version=\"1.0\"?>\n<!-- Copyright -->\n\
        <assembly xmlns=\"urn:schemas-microsoft-com:asm.v1\" manifestVersion=\"1.0\">\n\
        <noInheritable/>\n  <assemblyIdentity\n    type=\"win32\"\n    name=\"Microsoft.VC90.CRT\"\n\
        \x20   version=\"9.0.30729.6161\"\n    processorArchitecture=\"x86\"\n\
        \x20   publicKeyToken=\"1fc8b3b9a1e18e3b\"\n  />\n</assembly>";

    #[test]
    fn the_identity_is_read_from_a_runtime_manifest() {
        let identity = parse_identity(CRT_MANIFEST).expect("an identity");
        assert_eq!(identity.name, "Microsoft.VC90.CRT");
        assert_eq!(identity.version, "9.0.30729.6161");
        assert_eq!(identity.architecture, "x86");
        assert_eq!(identity.public_key_token, "1fc8b3b9a1e18e3b");
        assert!(parse_identity("<assembly/>").is_none());
    }

    #[test]
    fn the_generated_manifest_declares_each_runtime_as_a_dependency() {
        let identity = parse_identity(CRT_MANIFEST).unwrap();
        let text = dependency_manifest(std::slice::from_ref(&identity));
        assert!(text.contains("<dependentAssembly>"));
        assert!(text.contains("name=\"Microsoft.VC90.CRT\""));
        assert!(text.contains("version=\"9.0.30729.6161\""));
        assert_eq!(parse_identity(&text), Some(identity));
    }

    #[test]
    fn runtimes_are_found_beside_the_plugin_or_one_level_up() {
        let root = std::env::temp_dir().join(format!("rustamp-sxs-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let plugins = root.join("Plugins");
        let crt = root.join("Microsoft.VC90.CRT");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::create_dir_all(&crt).unwrap();
        std::fs::write(crt.join("Microsoft.VC90.CRT.manifest"), CRT_MANIFEST).unwrap();
        // A folder with a similar name but no manifest is not a runtime.
        std::fs::create_dir_all(root.join("Microsoft.VC80.CRT")).unwrap();

        let (dir, found) = find_runtimes(&[plugins.as_path(), root.as_path()]).unwrap();
        assert_eq!(dir, root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Microsoft.VC90.CRT");
        assert!(find_runtimes(&[plugins.as_path()]).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
