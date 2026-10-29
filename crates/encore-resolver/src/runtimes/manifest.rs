//! Pinned player components (D22): exact versions, official HTTPS URLs,
//! sizes and SHA-256 checksums. Every download is checked against these
//! before it is used.
//!
//! To update a pin, change the version, URL, size and checksum together; the
//! checksums come from the projects' own release files (Python:
//! `SHA256SUMS` of the python-build-standalone release; Deno:
//! `deno-<target>.zip.sha256sum`; wheels: PyPI's `sha256` digest). Encore
//! downloads new pins on launch for users who already approved the
//! components.

use sha2::{Digest, Sha256};

/// Bumped when the way components are unpacked changes, so existing installs
/// are redone.
pub const LAYOUT: u32 = 1;

pub const PYTHON_VERSION: &str = "3.13.18";
const PYTHON_BUILD: &str = "20261026";
pub const DENO_VERSION: &str = "2.9.8";
pub const YT_DLP_VERSION: &str = "2026.10.26";
pub const YT_DLP_EJS_VERSION: &str = "0.8.0";
pub const CERTIFI_VERSION: &str = "2026.10.23";

/// Targets Encore has pins for.
pub const TARGETS: &[&str] = &["aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];

/// (target, sha256, download bytes, approximate bytes on disk once unpacked)
const PYTHON: &[(&str, &str, u64, u64)] = &[
    ("aarch64-apple-darwin", "8392b6ea51d94391befb2c31ebe75463f045d0b69dfcf5e1bb3234e1f5fa3405", 25_265_774, 30_000_000),
    ("x86_64-apple-darwin", "7412c19d896c99431b74a18a5643e110449a7f9590fbab179c3e5ab27b9aa9e3", 25_055_263, 30_000_000),
    ("x86_64-pc-windows-msvc", "7369f7a35f261222a6ac8b3a87c1c88eeebee8af945d85346202591a64038dd3", 22_023_022, 40_000_000),
    ("x86_64-unknown-linux-gnu", "df15fe60d3ff55bf9dcf25824b5572756bb8816ff93865a273d3301d7066a8b7", 35_395_906, 60_000_000),
    ("aarch64-unknown-linux-gnu", "578d9da1983463ffb0c8c98ab0da2f18177f44069328eb0af847860c1eb05c3e", 29_492_455, 55_000_000),
];

const DENO: &[(&str, &str, u64, u64)] = &[
    ("aarch64-apple-darwin", "9f0bf660393da2eaff044cdc7444cb08c73f235849b61d224d825a194ce7c98e", 38_770_795, 81_512_928),
    ("x86_64-apple-darwin", "4b2f5edd2639b7470f034238b321d7002d8dfb4726d8558f52c17afac370eb2c", 42_797_031, 90_000_000),
    ("x86_64-pc-windows-msvc", "cba84b26e0df5ae7f74fc66b86a95b866872a86292ebd84fa1e959927edd93bf", 42_917_472, 95_000_000),
    ("x86_64-unknown-linux-gnu", "e890975bc2549807bbcc0b584ffafda72cc7c93349dd99c80483dd784f6c4609", 41_963_064, 95_000_000),
    ("aarch64-unknown-linux-gnu", "d49596c20b9788cf8f5d8ddf644103d4bde56ed239753f0bdb92ba9361d51905", 40_120_596, 90_000_000),
];

/// (file name, URL, sha256, bytes)
const WHEELS: &[(&str, &str, &str, u64)] = &[
    (
        "yt_dlp-2026.10.26-py3-none-any.whl",
        "https://files.pythonhosted.org/packages/7f/3d/16ea7d5638cf585991f2c5705e8083688dde633d32d0871f2404e0f4d827/yt_dlp-2026.10.26-py3-none-any.whl",
        "a05c5b5d8562f7e8a62f5334348b406af4930d56d3a65f8021d14af0e22339ae",
        3_186_201,
    ),
    (
        "yt_dlp_ejs-0.8.0-py3-none-any.whl",
        "https://files.pythonhosted.org/packages/e3/bd/520769863744b669440a924271a6159ddd82ad5ae26b4ac4d4b69e9f8d44/yt_dlp_ejs-0.8.0-py3-none-any.whl",
        "79300e5fca7f937a1eeede11f0456862c1b41107ce1d726871e0207424f4bdb4",
        53_443,
    ),
    (
        "certifi-2026.10.23-py3-none-any.whl",
        "https://files.pythonhosted.org/packages/32/d1/a3652fe2c19134d0d15922177821d863cbd7e842d53a21bbe7e8e3a1b5ac/certifi-2026.10.23-py3-none-any.whl",
        "e9b54de76e530bb6cf0ccd036787103acb58fd42ba930ea8bf948ddbaa2f61d1",
        135_082,
    ),
];
/// yt-dlp, its EJS scripts and certifi unpacked, plus the bytecode Python
/// writes on first import.
const PACKAGES_INSTALLED: u64 = 17_000_000;

/// Hosts downloads may start from. Redirects (GitHub's release CDN) must
/// stay on HTTPS; the checksum is what makes a file trusted.
pub const ALLOWED_HOSTS: &[&str] = &["github.com", "files.pythonhosted.org"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Python,
    Deno,
    Packages,
}

impl Kind {
    pub fn slug(self) -> &'static str {
        match self {
            Kind::Python => "python",
            Kind::Deno => "deno",
            Kind::Packages => "yt-dlp",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Python => "Python",
            Kind::Deno => "Deno",
            Kind::Packages => "yt-dlp",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    TarGz,
    Zip,
    /// A pure-Python wheel: a zip whose `*.data/` directory is skipped.
    Wheel,
}

#[derive(Clone, Debug)]
pub struct Artifact {
    /// File name, for logs.
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub format: Format,
}

#[derive(Clone, Debug)]
pub struct Component {
    pub kind: Kind,
    pub version: String,
    pub files: Vec<Artifact>,
    /// Approximate bytes on disk once unpacked.
    pub installed: u64,
}

impl Component {
    /// Identifies exactly what is installed: the layout and every checksum.
    pub fn fingerprint(&self) -> String {
        let mut h = Sha256::new();
        h.update(format!("layout={LAYOUT};kind={};version={}", self.kind.slug(), self.version));
        for f in &self.files {
            h.update(b";");
            h.update(f.sha256.as_bytes());
        }
        hex::encode(h.finalize())
    }

    /// Directory name under the runtimes folder.
    pub fn id(&self) -> String {
        format!("{}-{}-{}", self.kind.slug(), self.version, &self.fingerprint()[..10])
    }

    pub fn download_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

#[derive(Clone, Debug)]
pub struct Manifest {
    pub target: String,
    pub components: Vec<Component>,
}

impl Manifest {
    /// The pins for the system Encore is running on.
    pub fn pinned() -> Result<Manifest, String> {
        let target = current_target().ok_or_else(|| format!("Playback isn't available on this system yet ({} {}).", std::env::consts::OS, std::env::consts::ARCH))?;
        Self::for_target(target).ok_or_else(|| format!("Playback isn't available on {target} yet."))
    }

    pub fn for_target(target: &str) -> Option<Manifest> {
        let py = PYTHON.iter().find(|p| p.0 == target)?;
        let deno = DENO.iter().find(|p| p.0 == target)?;
        let py_name = format!("cpython-{PYTHON_VERSION}+{PYTHON_BUILD}-{target}-install_only_stripped.tar.gz");
        let python = Component {
            kind: Kind::Python,
            version: PYTHON_VERSION.into(),
            files: vec![Artifact {
                url: format!("https://github.com/astral-sh/python-build-standalone/releases/download/{PYTHON_BUILD}/{}", py_name.replace('+', "%2B")),
                name: py_name,
                sha256: py.1.into(),
                size: py.2,
                format: Format::TarGz,
            }],
            installed: py.3,
        };
        let deno_name = format!("deno-{target}.zip");
        let deno = Component {
            kind: Kind::Deno,
            version: DENO_VERSION.into(),
            files: vec![Artifact { url: format!("https://github.com/denoland/deno/releases/download/v{DENO_VERSION}/{deno_name}"), name: deno_name, sha256: deno.1.into(), size: deno.2, format: Format::Zip }],
            installed: deno.3,
        };
        let packages = Component {
            kind: Kind::Packages,
            version: YT_DLP_VERSION.into(),
            files: WHEELS.iter().map(|w| Artifact { name: w.0.into(), url: w.1.into(), sha256: w.2.into(), size: w.3, format: Format::Wheel }).collect(),
            installed: PACKAGES_INSTALLED,
        };
        Some(Manifest { target: target.into(), components: vec![python, deno, packages] })
    }

    pub fn get(&self, kind: Kind) -> Option<&Component> {
        self.components.iter().find(|c| c.kind == kind)
    }
}

/// The pin target for this process (an x86_64 build under Rosetta uses the
/// x86_64 components, which is what it can run).
pub const fn current_target() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("aarch64-apple-darwin")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("x86_64-apple-darwin")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("x86_64-pc-windows-msvc")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("x86_64-unknown-linux-gnu")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("aarch64-unknown-linux-gnu")
    } else {
        None
    }
}

/// Python's executable inside its component folder.
pub fn python_exe(target: &str) -> &'static str {
    if target.contains("windows") { "python/python.exe" } else { "python/bin/python3" }
}

pub fn deno_exe(target: &str) -> &'static str {
    if target.contains("windows") { "deno.exe" } else { "deno" }
}

/// Parts of the Python distribution playback never uses (IDLE, Tk, pip,
/// headers, the test suite, the embedding library on macOS, whose python
/// executable is linked statically). Skipping them roughly halves the size
/// on disk. `parts` are the archive path's components.
pub fn python_skip(target: &str, parts: &[&str]) -> bool {
    let windows = target.contains("windows");
    let macos = target.contains("apple");
    let p: Vec<String> = parts.iter().map(|s| if windows { s.to_ascii_lowercase() } else { s.to_string() }).collect();
    let p: Vec<&str> = p.iter().map(String::as_str).collect();
    // Everything lives under `python/`.
    let [first, rest @ ..] = p.as_slice() else { return false };
    if *first != "python" {
        return false;
    }
    const STDLIB_SKIP: &[&str] = &["idlelib", "tkinter", "turtledemo", "ensurepip", "pydoc_data", "test", "__phello__", "lib2to3"];
    let stdlib_skip = |name: &str, more: &[&str]| -> bool {
        STDLIB_SKIP.contains(&name) || name.starts_with("config-3") || name == "turtle.py" || (name == "site-packages" && more.first().is_some_and(|m| m.starts_with("pip")))
    };
    match rest {
        [top, ..] if ["include", "share"].contains(top) => true,
        // Windows: python/{Lib,DLLs,libs,Scripts,tcl,include}
        [top, ..] if windows && ["libs", "scripts", "tcl"].contains(top) => true,
        ["dlls", f] if windows => f.starts_with("tcl") || f.starts_with("tk") || f.starts_with("_tkinter"),
        ["lib", name, more @ ..] if windows => stdlib_skip(name, more),
        // Unix: python/bin, python/lib/python3.x/..., python/lib/<libs>
        ["bin", f] => f.starts_with("idle") || f.starts_with("pip") || f.starts_with("pydoc") || f.ends_with("-config"),
        ["lib", lib, name, more @ ..] if lib.starts_with("python3") => stdlib_skip(name, more) || (*name == "lib-dynload" && more.first().is_some_and(|m| m.starts_with("_tkinter"))),
        ["lib", f, ..] => {
            ["tcl", "tk", "itcl", "thread", "libtcl", "libtk", "pkgconfig"].iter().any(|s| f.starts_with(s)) || (macos && f.starts_with("libpython3") && f.ends_with(".dylib"))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_target_has_pins() {
        for t in TARGETS {
            let m = Manifest::for_target(t).unwrap_or_else(|| panic!("no pins for {t}"));
            assert_eq!(m.components.len(), 3);
            for c in &m.components {
                for f in &c.files {
                    assert!(f.url.starts_with("https://"), "{}", f.url);
                    let host = f.url.trim_start_matches("https://").split('/').next().unwrap();
                    assert!(ALLOWED_HOSTS.contains(&host), "{host}");
                    assert_eq!(f.sha256.len(), 64);
                    assert!(f.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()), "lower-case hex: {}", f.sha256);
                    assert!(f.size > 10_000);
                }
            }
        }
        assert!(current_target().is_none_or(|t| TARGETS.contains(&t)));
    }

    #[test]
    fn ids_change_with_checksums() {
        let a = Manifest::for_target("x86_64-unknown-linux-gnu").unwrap();
        let mut b = a.clone();
        b.components[0].files[0].sha256 = "0".repeat(64);
        assert_ne!(a.components[0].id(), b.components[0].id());
        assert_eq!(a.components[1].id(), b.components[1].id());
        assert!(a.components[0].id().starts_with("python-3.13.18-"));
    }

    #[test]
    fn python_trim_rules() {
        let mac = "aarch64-apple-darwin";
        let s = |p: &str| python_skip(mac, &p.split('/').collect::<Vec<_>>());
        assert!(s("python/lib/python3.13/idlelib/foo.py"));
        assert!(s("python/lib/python3.13/site-packages/pip/__init__.py"));
        assert!(s("python/lib/libpython3.13.dylib"));
        assert!(s("python/bin/pip3"));
        assert!(s("python/include/python3.13/Python.h"));
        assert!(!s("python/bin/python3.13"));
        assert!(!s("python/lib/python3.13/json/__init__.py"));
        assert!(!s("python/lib/python3.13/site-packages/README.txt"));
        assert!(!s("python/lib/python3.13/lib-dynload/_dbm.cpython-313-darwin.so"));
        let lin = "x86_64-unknown-linux-gnu";
        assert!(!python_skip(lin, &["python", "lib", "libpython3.13.so.1.0"]));
        let win = "x86_64-pc-windows-msvc";
        let w = |p: &str| python_skip(win, &p.split('/').collect::<Vec<_>>());
        assert!(w("python/Lib/idlelib/x.py"));
        assert!(w("python/Lib/site-packages/pip/x.py"));
        assert!(w("python/tcl/tk8.6/x.tcl"));
        assert!(w("python/DLLs/tcl86t.dll"));
        assert!(!w("python/python.exe"));
        assert!(!w("python/python313.dll"));
        assert!(!w("python/Lib/json/__init__.py"));
        assert!(!w("python/DLLs/_ssl.pyd"));
    }
}
