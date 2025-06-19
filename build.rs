// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::process::Command;
use std::path::Path;
use std::env;
use walkdir::WalkDir;
use std::fmt::Write;

fn compile_qml(dir: &str, qt_include_path: &str, qt_library_path: &str) {
    let mut config = cc::Build::new();
    config.include(qt_include_path);
    config.include(&format!("{}/QtCore", qt_include_path));
    config.include(&format!("{}/QtQml", qt_include_path));
    if cfg!(target_os = "macos") {
        config.include(format!("{}/QtCore.framework/Headers/", qt_library_path));
        config.include(format!("{}/QtQml.framework/Headers/", qt_library_path));
    }
    for f in env::var("DEP_QT_COMPILE_FLAGS").unwrap().split_terminator(';') {
        config.flag(f);
    }

    println!("cargo:rerun-if-changed={}", dir);

    let out_dir = env::var("OUT_DIR").unwrap();
    let out_dir = Path::new(&out_dir);
    let main_dir = env::var("CARGO_MANIFEST_DIR").unwrap();

    let mut files = Vec::new();
    let mut qrc = "<RCC>\n<qresource prefix=\"/\">\n".to_string();
    WalkDir::new(dir).into_iter().flatten().for_each(|entry| {
        let f_name = entry.path().to_string_lossy().replace('\\', "/");
        if f_name.ends_with(".qml") || f_name.ends_with(".js") {
            let _ = writeln!(qrc, "<file>{}</file>", f_name);

            let cpp_name = f_name.replace('/', "_").replace(".qml", ".cpp").replace(".js", ".cpp");
            let cpp_path = out_dir.join(cpp_name).to_string_lossy().to_string();

            config.file(&cpp_path);
            files.push((f_name, cpp_path));
        }
    });

    let qt_path = Path::new(qt_library_path).parent().unwrap();
    let compiler_path = if qt_path.join("libexec/qmlcachegen").exists() {
        qt_path.join("libexec/qmlcachegen").to_string_lossy().to_string()
    } else if qt_path.join("../macos/libexec/qmlcachegen").exists() {
        qt_path.join("../macos/libexec/qmlcachegen").to_string_lossy().to_string()
    } else if env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" && env::var("CARGO_CFG_TARGET_ARCH").unwrap() == "aarch64" {
        qt_path.join("../msvc2019_64/bin/qmlcachegen").to_string_lossy().to_string()
    } else {
        "qmlcachegen".to_string()
    };

    qrc.push_str("</qresource>\n</RCC>");
    let qrc_path = Path::new(&main_dir).join("ui.qrc").to_string_lossy().to_string();
    std::fs::write(&qrc_path, qrc).unwrap();

    for (qml, cpp) in &files {
        assert!(Command::new(&compiler_path).args(["--resource", &qrc_path, "-o", cpp, qml]).status().unwrap().success());
    }

    let loader_path = out_dir.join("qmlcache_loader.cpp").to_str().unwrap().to_string();
    assert!(Command::new(&compiler_path).args(["--resource-file-mapping", &qrc_path, "-o", &loader_path, "ui.qrc"]).status().unwrap().success());

    config.file(&loader_path);

    std::fs::remove_file(&qrc_path).unwrap();

    config.cargo_metadata(false).compile("qmlcache");
    println!("cargo:rustc-link-lib=static:+whole-archive=qmlcache");
}

fn main() {
    // 这些DEP_*环境变量通常由依赖crate提供，比如qmetaobject或qttypes库的构建脚本设置
    let qt_include_path = env::var("DEP_QT_INCLUDE_PATH").unwrap();
    let qt_library_path = env::var("DEP_QT_LIBRARY_PATH").unwrap();
    let qt_version = env::var("DEP_QT_VERSION").unwrap();

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap(); // 这个环境变量由Cargo设置，表示当前的目标操作系统

    // OUT_DIR是Cargo自动提供的环境变量，表示构建输出目录，判断是否成功获取路径
    if let Ok(out_dir) = env::var("OUT_DIR") {
        // 告诉Rust编译器，项目中存在一个名为compiled_qml的配置选项，允许代码中使用 #[cfg(compiled_qml)]
        println!("cargo::rustc-check-cfg=cfg(compiled_qml)");
        // 判断当前构建是否处于部署路径或构建的是移动平台（Android或iOS）
        if out_dir.contains("\\deploy\\build\\") || out_dir.contains("/deploy/build/") || target_os == "android" || target_os == "ios" {
            // 调用自定义函数编译QML文件
            compile_qml("src/ui/", &qt_include_path, &qt_library_path);
            println!("cargo:rustc-cfg=compiled_qml");
        }
    }

    let mut config = cpp_build::Config::new(); // 创建一个新的C++编译配置对象

    // 读取环境变量DEP_QT_COMPILE_FLAGS，通常由qt_build.rs或上游crate提供
    for f in env::var("DEP_QT_COMPILE_FLAGS").unwrap().split_terminator(';') {
        config.flag(f);
    }
    // config.define("QT_QML_DEBUG", None);
    // 告诉Cargo：如果src/qt_gpu/qrhi_undistort.cpp发生变化，就重新运行build.rs脚本并重新编译。否则Cargo会缓存构建结果，不重新生成
    println!("cargo:rerun-if-changed=src/qt_gpu/qrhi_undistort.cpp");

    if target_os == "ios" {
        println!("cargo:rerun-if-changed=_deployment/ios/qml_plugins.cpp");
        config.file("_deployment/ios/qml_plugins.cpp");

        println!("cargo:rustc-link-arg=-Wl,-e,_qt_main_wrapper");
        println!("cargo:rustc-link-arg=-fapple-link-rtlib");
        println!("cargo:rustc-link-arg=-dead_strip");

        let frameworks = [
            "AudioToolbox", "AVFoundation", "CoreAudio", "CoreFoundation",
            "CoreGraphics", "CoreMedia", "CoreServices", "CoreText",
            "CoreVideo", "Foundation", "ImageIO", "IOKit", "CFNetwork",
            "OpenGLES", "QuartzCore", "Security", "SystemConfiguration",
            "UIKit", "UniformTypeIdentifiers", "VideoToolbox", "Photos"
        ];

        println!("cargo:rustc-link-lib=z");
        println!("cargo:rustc-link-lib=bz2");
        println!("cargo:rustc-link-lib=xml2");
        for x in frameworks {
            println!("cargo:rustc-link-lib=framework={x}");
        }

        let mut added_paths = vec![];
        for x in walkdir::WalkDir::new(Path::new(&qt_library_path).parent().unwrap()) {
            let x = x.unwrap();
            let name = x.file_name().to_str().unwrap();
            let path = x.path().to_str().unwrap();
            if path.contains("objects-Debug") ||
               path.contains("Imagine") ||
               path.contains("Fusion") ||
               path.contains("Universal") ||
               path.to_ascii_lowercase().contains("particles") ||
               path.to_ascii_lowercase().contains("tooling") ||
               path.to_ascii_lowercase().contains("test") {
                continue;
            }
            if name.starts_with("qrc_") && name.ends_with(".cpp.o") {
                println!("cargo:rustc-link-arg=-force_load");
                println!("cargo:rustc-link-arg={}", path);
            }
            if name.starts_with("lib") && name.ends_with(".a") {
                let parent_path = x.path().parent().unwrap().to_str().unwrap().to_owned();
                if !added_paths.contains(&parent_path) {
                    println!("cargo:rustc-link-search={}", parent_path);
                    added_paths.push(parent_path);
                }
                if !name.contains("_debug") && !name.contains("Widgets") && !name.contains("Test") {
                    println!("cargo:rustc-link-lib={}", name[3..].replace(".a", ""));
                }
            }
        };
    } else if target_os == "macos" {
        println!("cargo:rustc-link-lib=z");
        println!("cargo:rustc-link-lib=bz2");
        println!("cargo:rustc-link-lib=xml2");
        println!("cargo:rustc-link-lib=framework=AudioToolbox");
        println!("cargo:rustc-link-lib=framework=VideoToolbox");
        println!("cargo:rustc-link-lib=framework=QuartzCore");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=CoreMedia");
        println!("cargo:rustc-link-lib=framework=CoreAudio");
        println!("cargo:rustc-link-lib=framework=CoreVideo");
        println!("cargo:rustc-link-lib=framework=CoreServices");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=OpenGL");
        println!("cargo:rustc-link-lib=framework=CFNetwork");
        println!("cargo:rustc-link-lib=framework=Security");
    }

    // 定义一个可变闭包public_include，它接受一个参数name（例如QtCore，QtGui），闭包体中为每一个模块名设置对应的头文件路径
    let mut public_include = |name| {
        // 特殊处理macOS平台的头文件路径
        if cfg!(target_os = "macos") {
            config.include(format!("{}/{}.framework/Headers/", qt_library_path, name));
        }
        config.include(format!("{}/{}", qt_include_path, name));
    };
    // 应用public_include闭包，为每个Qt模块添加头文件路径
    public_include("QtCore");
    public_include("QtGui");
    public_include("QtQuick");
    public_include("QtQml");
    public_include("QtQuickControls2");

    let mut private_include = |name| {
        if cfg!(target_os = "macos") {
            config.include(format!("{}/{}.framework/Headers/{}",       qt_library_path, name, qt_version));
            config.include(format!("{}/{}.framework/Headers/{}/{}",    qt_library_path, name, qt_version, name));
        }
        config.include(format!("{}/{}/{}", qt_include_path, name, qt_version))
              .include(format!("{}/{}/{}/{}", qt_include_path, name, qt_version, name));
    };
    private_include("QtCore");
    private_include("QtGui");
    private_include("QtQuick");
    private_include("QtQml");

    match target_os.as_str() {
        "android" => {
            println!("cargo:rustc-link-search={}/lib/arm64-v8a", env::var("FFMPEG_DIR").unwrap());
            println!("cargo:rustc-link-search={}/lib", env::var("FFMPEG_DIR").unwrap());
            config.include(format!("{}/include", env::var("FFMPEG_DIR").unwrap()));
        },
        "macos" | "ios" => {
            println!("cargo:rustc-link-search={}/lib", env::var("FFMPEG_DIR").unwrap());
            println!("cargo:rustc-link-lib=static:+whole-archive=x264");
            println!("cargo:rustc-link-lib=static=x265");
        },
        "linux" => {
            // 从环境变量OPENCV_LINK_PATHS中读取OpenCV库的链接路径
            println!("cargo:rustc-link-search={}", env::var("OPENCV_LINK_PATHS").unwrap());
            // 设置FFmpeg的架构路径，FFMPEG_ARCH是可选的，默认为amd64
            println!("cargo:rustc-link-search={}/lib/{}", env::var("FFMPEG_DIR").unwrap(), env::var("FFMPEG_ARCH").unwrap_or("amd64".into()));
            println!("cargo:rustc-link-search={}/lib", env::var("FFMPEG_DIR").unwrap());
            println!("cargo:rustc-link-lib=static:+whole-archive=z");
            // 区分是否是使用vcpkg安装的OpenCV库
            if env::var("OPENCV_LINK_PATHS").unwrap_or_default().contains("vcpkg") {
                // 如果是vcpkg安装的OpenCV库，采用静态链接，+whole-archive确保所有符号都包含
                env::var("OPENCV_LINK_LIBS").unwrap().split(',').for_each(|lib| println!("cargo:rustc-link-lib=static:+whole-archive={}", lib.trim()));
            } else {
                env::var("OPENCV_LINK_LIBS").unwrap().split(',').for_each(|lib| println!("cargo:rustc-link-lib={}", lib.trim()));
            }
        },
        "windows" => {
            println!("cargo:rustc-link-arg=/EXPORT:NvOptimusEnablement");
            println!("cargo:rustc-link-arg=/EXPORT:AmdPowerXpressRequestHighPerformance");
            println!("cargo:rustc-link-search={}", env::var("OPENCV_LINK_PATHS").unwrap());
            println!("cargo:rustc-link-search={}\\lib\\{}", env::var("FFMPEG_DIR").unwrap(), env::var("FFMPEG_ARCH").unwrap_or("x64".into()));
            println!("cargo:rustc-link-search={}\\lib", env::var("FFMPEG_DIR").unwrap());
            let mut res = winres::WindowsResource::new();
            res.set_icon("resources/app_icon.ico");
            res.set("FileVersion", env!("CARGO_PKG_VERSION"));
            res.set("ProductVersion", env!("CARGO_PKG_VERSION"));
            res.set("ProductName", "Gyroflow");
            res.set("FileDescription", &format!("Gyroflow v{}", env!("CARGO_PKG_VERSION")));
            res.compile().unwrap();
        }
        tos => panic!("unknown target os {:?}!", tos)
    }

    // 根据当前时间生成一个变化频率为10分钟的版本标识值，设置为编译时环境变量BUILD_TIME
    if let Ok(time) = std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH) {
        println!("cargo:rustc-env=BUILD_TIME={}", (time.as_secs() - 1642516578) / 600); // New version every 10 minutes
    }

    config
        .include(&qt_include_path)
        .build("src/gyroflow.rs");

    if target_os == "ios" {
        let out_dir = env::var("OUT_DIR").unwrap();
        for entry in Path::new(&out_dir).read_dir().unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() && path.to_string_lossy().contains("qml_plugins.o") {
                println!("cargo:rustc-link-arg=-force_load");
                println!("cargo:rustc-link-arg={}", path.to_string_lossy());
                break;
            }
        }
    }
}
