//! Temporary case roots under `<workspace root>/target/check-tmp`, and their removal.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// Where case roots are built: `<workspace root>/target/check-tmp`. Not the system temp
/// directory, because on the development machine project-built executables may not write there.
pub fn case_tmp_base() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest.parent().and_then(Path::parent).unwrap_or(manifest);
    workspace.join("target").join("check-tmp")
}

/// A fresh, empty directory under `case_tmp_base()`.
pub fn fresh_root() -> std::io::Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let root = case_tmp_base().join(format!("check-case-{}-{n}-{nanos}", std::process::id()));
    fs::create_dir_all(&root)
        .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", root.display())))?;
    Ok(root)
}

/// Removes a tree, first clearing read-only flags (git object files are read-only on Windows).
pub fn remove_tree(root: &Path) -> std::io::Result<()> {
    fn writable(p: &Path) {
        let Ok(meta) = fs::symlink_metadata(p) else {
            return;
        };
        if meta.is_dir()
            && let Ok(rd) = fs::read_dir(p)
        {
            for e in rd.flatten() {
                writable(&e.path());
            }
        }
        let mut perms = meta.permissions();
        if perms.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = fs::set_permissions(p, perms);
        }
    }
    writable(root);
    fs::remove_dir_all(root)
}
