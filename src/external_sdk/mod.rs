// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

mod braw;
pub mod r3d;
mod ffmpeg_gpl;

pub use ffmpeg_gpl::FfmpegGpl;

use std::io::*;
use std::io;
use flate2::read::GzDecoder;

// 使用lazy_static这个非常流行的第三方库（crate）来实现全局变量的懒加载初始化。
// 虽然现代Rust(1.51+)已经内置了std::sync::OnceLock，可以实现类似的功能
// 创建一个名为SDK_PATH的全局静态变量，它的值通过调用get_sdk_path()函数来初始化，但这个初始化操作只会在SDK_PATH 
// 第一次被访问时执行一次，并且后续的所有访问都会直接返回第一次计算出的结果
lazy_static::lazy_static! {
    pub static ref SDK_PATH: Result<std::path::PathBuf> = get_sdk_path();
}

// 获取应用程序依赖库的路径
fn get_sdk_path() -> Result<std::path::PathBuf> {
    // current_exe()函数返回当前可执行文件的路径，ok_or_else将Option转换为Result
    let mut out_dir = std::env::current_exe()?.parent().ok_or_else(|| Error::new(ErrorKind::Other, "Cannot get exe parent"))?.to_path_buf();
    if cfg!(target_os = "macos") {
        out_dir.push("../Frameworks/");
    }
    if cfg!(target_os = "linux") {
        out_dir.push("lib/");
    }
    /*{
        let mut test = out_dir.clone();
        test.push("__tmp_test");
        let writable = std::fs::File::create(&test).is_ok();
        let _ = std::fs::remove_file(test);
        if !writable {
            // Get writeable path
            if let Some(new_dir) = directories::ProjectDirs::from("xyz", "Gyroflow", "Gyroflow") {
                let mut writable_path = new_dir.data_local_dir();
                if std::fs::create_dir_all(writable_path).is_ok() {
                    out_dir = writable_path.to_path_buf();
                }
            }
        }
    }*/
    Ok(out_dir)
}

pub fn requires_install(filename: &str) -> bool {
    if filename.to_lowercase().ends_with(".braw") { return !braw::BrawSdk::is_installed(); }
    if filename.to_lowercase().ends_with(".r3d") { return !r3d::REDSdk::is_installed(); }
    if filename == "ffmpeg_gpl" { return !FfmpegGpl::is_installed(); }
    false
}

pub fn install<F: Fn((f64, &'static str, String)) + Send + Sync + Clone + 'static>(filename: &str, cb: F) {
    let (url, sdk_name) = if filename.to_lowercase().ends_with(".braw") {
        (braw::BrawSdk::get_download_url(), "Blackmagic RAW SDK")
    } else if filename.to_lowercase().ends_with(".r3d") {
        (r3d::REDSdk::get_download_url(), "RED SDK")
    } else if filename == "ffmpeg_gpl" {
        (FfmpegGpl::get_download_url(), "FFmpeg GPL codecs (x264, x265)")
    } else {
        (None, "")
    };

    if let Some(url) = url {
        crate::core::run_threaded(move || {
            let result = (|| -> Result<()> {
                log::info!("Downloading from {}", url);
                let reader = ureq::get(url).call().map_err(|e| Error::new(ErrorKind::Other, e.to_string()))?;
                let size = reader.headers().get("content-length").and_then(|x| x.to_str().unwrap().parse::<usize>().ok()).unwrap_or(1).max(1);
                let mut reader = ProgressReader::new(reader.into_body().into_reader(), |read| {
                    cb(((read as f64 / size as f64) * 0.5, sdk_name, "".into()));
                });
                let mut buf = Vec::with_capacity(4*1024*1024);
                io::copy(&mut reader, &mut buf)?;

                let out_dir = SDK_PATH.as_ref().map_err(|e| Error::new(e.kind(), e))?;
                let size = buf.len().max(1) as f64;
                let br = ProgressReader::new(Cursor::new(buf), |read| {
                    cb((0.5 + (read as f64 / size) * 0.5, sdk_name, "".into()));
                });
                let gz = GzDecoder::new(br);
                let mut archive = tar::Archive::new(gz);
                'files: for file in archive.entries()? {
                    let mut file = file.unwrap();
                    let mut final_path = out_dir.clone();
                    for part in file.path()?.components() {
                        use std::path::Component;
                        match part {
                            Component::Prefix(..) | Component::RootDir | Component::CurDir => continue,
                            Component::ParentDir => continue 'files,
                            Component::Normal(part) => final_path.push(part),
                        }
                    }
                    if final_path.exists() {
                        let _ = std::fs::remove_file(&final_path);
                        if final_path.exists() {
                            let _ = std::fs::rename(&final_path, final_path.with_file_name(&format!("zz-remove-me-{}", final_path.file_name().unwrap().to_str().unwrap())));
                        }
                    }
                    file.unpack_in(out_dir)?;
                }
                Ok(())
            })();
            if let Err(e) = result {
                cb((1.0, sdk_name, e.to_string()));
            } else {
                cb((1.0, sdk_name, String::new()));
            }
        });
    } else {
        cb((1.0, sdk_name, "SDK is not available.".to_string()));
    }
}

// 清理标记为以"zz-remove-me-"开头的文件
pub fn cleanup() -> Result<()> {
    let mut out_dir = std::env::current_exe()?.parent().ok_or_else(|| Error::new(ErrorKind::Other, "Cannot get exe parent"))?.to_path_buf();
    if cfg!(target_os = "macos") {
        out_dir.push("../Frameworks/");
    }
    // 创建一个迭代器来遍历指定目录下的所有文件，flatten将迭代器中的结果扁平化
    walkdir::WalkDir::new(out_dir).into_iter().flatten().for_each(|entry| {
        let path = entry.path();
        if let Some(fname) = path.file_name() {
            // 检查文件名是否以"zz-remove-me-"开头
            if fname.to_str().unwrap_or("").starts_with("zz-remove-me-") {
                let _ = std::fs::remove_file(path); // 尝试删除该文件
            }
        }
    });
    Ok(())
}

pub struct ProgressReader<R: Read, C: FnMut(usize)> {
    reader: R,
    callback: C,
    total_read: usize
}
impl<R: Read, C: FnMut(usize)> ProgressReader<R, C> {
    pub fn new(reader: R, callback: C) -> Self {
        Self { reader, callback, total_read: 0 }
    }
}
impl<R: Read, C: FnMut(usize)> Read for ProgressReader<R, C> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.reader.read(buf)?;
        self.total_read += read;
        (self.callback)(self.total_read);
        Ok(read)
    }
}
