use std::path::{Path, PathBuf};

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_app_font)");
    println!("cargo::rustc-check-cfg=cfg(plugin_host)");
    // Third-party plugins (VST3, VST2, CLAP) are hosted on the desktop only: not on the web, Android or iOS.
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if arch != "wasm32" && os != "android" && os != "ios" {
        println!("cargo::rustc-cfg=plugin_host");
    }
    // `desktop`: the desktop sound modes, the MCP token and multi-core mixing (rayon, getrandom). `x11`: plugin editor
    // windows are X11 ones (x11rb). `native_midi`: midir has a MIDI input backend (its ALSA one is Linux only). Each
    // set matches its dependencies' target in Cargo.toml. `mobile`: the phones (Android, iOS), which have no file
    // dialogs, file manager or desktop sound library.
    for (name, systems) in [
        ("desktop", &["linux", "freebsd", "macos", "windows"][..]),
        ("x11", &["linux", "freebsd"]),
        ("native_midi", &["linux", "macos", "windows"]),
        ("mobile", &["android", "ios"]),
    ] {
        println!("cargo::rustc-check-cfg=cfg({name})");
        if systems.contains(&os.as_str()) {
            println!("cargo::rustc-cfg={name}");
        }
    }
    println!("cargo::rerun-if-changed=assets/fonts");
    if Path::new("assets/fonts/OpenSans_SemiCondensed-SemiBold.ttf").exists() {
        println!("cargo::rustc-cfg=has_app_font");
    }
    lemon_presets();
    video(&os);
}

/// With the `video` feature: compiles `src/video/shim.c` and links the static FFmpeg and x264 that
/// tools/build-ffmpeg.sh built into `$HONEYDEW_FFMPEG_DIR_<target triple, `-` as `_`>` (or `$HONEYDEW_FFMPEG_DIR`):
/// its `include` and `lib` folders.
fn video(os: &str) {
    if std::env::var_os("CARGO_FEATURE_VIDEO").is_none() {
        return;
    }
    let target = std::env::var("TARGET")
        .unwrap_or_default()
        .replace('-', "_");
    let key = format!("HONEYDEW_FFMPEG_DIR_{target}");
    println!("cargo::rerun-if-env-changed={key}");
    println!("cargo::rerun-if-env-changed=HONEYDEW_FFMPEG_DIR");
    println!("cargo::rerun-if-changed=src/video/shim.c");
    let dir = std::env::var_os(&key)
        .or_else(|| std::env::var_os("HONEYDEW_FFMPEG_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            panic!("the `video` feature needs FFmpeg: run tools/build-ffmpeg.sh and set {key} or HONEYDEW_FFMPEG_DIR")
        });
    cc::Build::new()
        .file("src/video/shim.c")
        .include(dir.join("include"))
        .warnings(false)
        .compile("honeydew_video");
    println!(
        "cargo::rustc-link-search=native={}",
        dir.join("lib").display()
    );
    for lib in [
        "avformat",
        "avcodec",
        "swscale",
        "swresample",
        "avutil",
        "x264",
    ] {
        println!("cargo::rustc-link-lib=static={lib}");
    }
    let system: &[&str] = match os {
        "android" => &["mediandk", "log", "m"],
        "windows" => &["bcrypt", "ole32", "user32"],
        "macos" | "ios" => &["iconv"],
        _ => &["m", "pthread", "dl"],
    };
    for lib in system {
        println!("cargo::rustc-link-lib={lib}");
    }
    if os == "macos" || os == "ios" {
        for framework in [
            "CoreFoundation",
            "CoreMedia",
            "CoreVideo",
            "VideoToolbox",
            "Security",
        ] {
            println!("cargo::rustc-link-lib=framework={framework}");
        }
    }
}

/// The Lemon plugins' presets (`assets/lemon-presets/<plugin>/[<category>/]<name>.ospreset`), built into the app: a
/// table of (path below that folder, bytes) that `plugins::presets` includes, sorted by path.
fn lemon_presets() {
    let root = Path::new("assets/lemon-presets");
    println!("cargo::rerun-if-changed=assets/lemon-presets");
    let mut files = Vec::new();
    collect_presets(root, &mut files);
    files.sort();
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let mut table = String::from("&[\n");
    for file in &files {
        let Ok(rel) = file.strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let abs = manifest.join(file);
        table += &format!(
            "    ({rel:?}, include_bytes!({:?}) as &[u8]),\n",
            abs.to_string_lossy()
        );
    }
    table += "]\n";
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    std::fs::write(out.join("lemon_presets.rs"), table).expect("the preset table writes");
}

fn collect_presets(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_presets(&path, files);
        } else if path.extension().is_some_and(|x| x == "ospreset") {
            files.push(path);
        }
    }
}
