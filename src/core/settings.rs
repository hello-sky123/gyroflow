// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Adrian <adrian.eddy at gmail>

use app_dirs2::{ AppDataType, AppInfo };
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::{ Arc, atomic::{ AtomicUsize, Ordering::SeqCst } };
use std::path::PathBuf;

// 获取应用程序数据目录，返回一个PathBuf(本地文件系统的路径)类型的路径
pub fn data_dir() -> PathBuf {
    static PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

    PATH.get_or_init(|| {
        // App_dirs2库提供了跨平台的应用程序数据目录获取方式，Linux下为/home/username/.local/share/应用名
        let mut path = app_dirs2::get_app_dir(AppDataType::UserData, &AppInfo { name: "Gyroflow", author: "Gyroflow" }, "").unwrap();
        if path.file_name().unwrap() == path.parent().unwrap().file_name().unwrap() {
            path = path.parent().unwrap().to_path_buf(); // 如果父目录和当前目录相同，进行修正（防止重复文件夹名）
        }

        #[cfg(target_os = "windows")]
        unsafe {
            use windows::Win32::UI::Shell::*;
            use std::os::windows::ffi::OsStringExt;
            let mut len = 0;
            let _ = windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName(&mut len, windows::core::PWSTR::null());
            if len > 0 {
                // It's a Microsoft Store package
                if let Ok(raw_path) = SHGetKnownFolderPath(&FOLDERID_Profile, KNOWN_FOLDER_FLAG::default(), None) {
                    let s = std::ffi::OsString::from_wide(raw_path.as_wide());
                    path = PathBuf::from(s);
                    path.push("AppData");
                    path.push("Local");
                    path.push("Gyroflow");
                    windows::Win32::System::Com::CoTaskMemFree(Some(raw_path.as_ptr() as *mut _));
                }
            }
        }

        #[cfg(target_os = "macos")]
        unsafe {
            use std::ffi::{CStr, OsString};
            use std::mem::MaybeUninit;
            use std::os::unix::ffi::OsStringExt;
            let init_size = match libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) {
                -1 => 1024,
                n => n as usize,
            };
            let mut buf = Vec::with_capacity(init_size);
            let mut pwd: MaybeUninit<libc::passwd> = MaybeUninit::uninit();
            let mut pwdp = std::ptr::null_mut();
            match libc::getpwuid_r(libc::geteuid(), pwd.as_mut_ptr(), buf.as_mut_ptr(), buf.capacity(), &mut pwdp) {
                0 if !pwdp.is_null() => {
                    let pwd = pwd.assume_init();
                    let bytes = CStr::from_ptr(pwd.pw_dir).to_bytes().to_vec();
                    let pw_dir = OsString::from_vec(bytes);
                    path = PathBuf::from(pw_dir);
                    path.push("Library");
                    path.push("Application Support");
                    path.push("Gyroflow");
                }
                _ => { },
            }
        }
        let _ = std::fs::create_dir_all(&path); // 递归创建目录
        if let Err(e) = std::fs::create_dir_all(&path.join("lens_profiles")) {
            ::log::error!("Failed to create lens profiles directory at {:?}: {e:?}", path.join("lens_profiles"));
        }
        path
    }).clone()
}

pub fn get_all() -> HashMap<String, serde_json::Value> {
    map().read().clone()
}

pub fn get(key: &str, default: serde_json::Value) -> serde_json::Value {
    map().read().get(key).unwrap_or(&default).clone()
}

// 接收一个键值对，将其更新到内存中的全局设置里，然后立即启动一个后台任务，将所有当前的设置异步地保存到磁盘文件中
pub fn set(key: &str, value: serde_json::Value) {
    // 获取全局map的引用，并请求一个写锁，.write()方法会阻塞直到获取到锁
    map().write().insert(key.to_string(), value); // 更新或插入键值对
    spawn_store_thread(); // 在内存中的数据更新后，调用我们的智能调度器来安排一次磁盘保存
}

pub fn contains(key: &str) -> bool {
    map().read().contains_key(key)
}

pub fn clear() {
    map().write().clear();
    store();
}
pub fn flush() {
    store();
}

pub fn try_get(key: &str) -> Option<serde_json::Value> { map().read().get(key).map(Clone::clone) }
pub fn get_u64(key: &str, default: u64) -> u64 { map().read().get(key).and_then(|x| x.as_u64()).unwrap_or(default) }
pub fn get_f64(key: &str, default: f64) -> f64 { map().read().get(key).and_then(|x| x.as_f64()).unwrap_or(default) }
pub fn get_bool(key: &str, default: bool) -> bool { map().read().get(key).and_then(|x| x.as_bool()).unwrap_or(default) }
pub fn get_str(key: &str, default: &str) -> String { map().read().get(key).and_then(|x| x.as_str()).map(|x| x.to_owned()).unwrap_or_else(|| default.to_owned()) }

// 返回一个对全局设置map的共享、线程安全的引用，serde_json::Value是一个任意JSON值的字典，HashMap哈希表
// RwLock读写锁，允许多个读者或一个写者同时访问数据，Arc是一个原子引用计数的智能指针，用于在多线程环境中共享数据
fn map() -> Arc<RwLock<HashMap<String, serde_json::Value>>> {
    // 声明一个静态的、只能被写入一次的全局变量。OnceLock是现代Rust中实现“单例模式”或“一次性全局初始化”最安全、最推荐的方式。
    // 整个Arc<RwLock<...>>结构将在第一次被需要时创建，且只创建一次
    static MAP: std::sync::OnceLock<Arc<RwLock<HashMap<String, serde_json::Value>>>> = std::sync::OnceLock::new();
    // 如果MAP没有初始化，就调用 || { ... }初始化一次,如果已初始化，直接返回已有值
    MAP.get_or_init(|| {
        // 闭包内的初始化逻辑
        let mut map = HashMap::new();
        // data_dir()获取特定于平台的应用数据目录
        let file = data_dir().join("settings.json"); // 拼接出settings.json文件的完整路径
        log::info!("Settings file path: {}", file.display());

        // 尝试从settings.json文件中读取JSON数据，并解析为HashMap<String, serde_json::Value>
        if let Ok(v) = serde_json::from_str::<HashMap<String, serde_json::Value>>(&std::fs::read_to_string(file).unwrap_or_default()) {
            map = v; // 如果读取成功，将其赋值给map
        }

        Arc::new(RwLock::new(map)) // 返回一个新的Arc<RwLock<HashMap<...>>>，它包含了读取的设置数据，或者为空
    }).clone() // 只增加引用计数，可以在多线程环境中安全地共享和修改设置数据
}

fn timestamp() -> usize { std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH).unwrap().as_secs() as usize }

fn spawn_store_thread() {
    // AtomicUsize是一个原子类型，提供了线程安全的整数操作
    static STORE_TIMEOUT: AtomicUsize = AtomicUsize::new(0);

    let is_thread_running = STORE_TIMEOUT.load(SeqCst) != 0; // 以最强的内存顺序加载STORE_TIMEOUT
    STORE_TIMEOUT.store(timestamp() + 1, SeqCst); // 1 second

    if is_thread_running { return; }
    // 只有在没有保存线程运行时，才会创建一个新的线程来处理保存操作
    std::thread::spawn(|| {
        // 这个线程会一直循环，直到计划的保存时间到来
        while STORE_TIMEOUT.load(SeqCst) > timestamp() {
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        store();
        STORE_TIMEOUT.store(0, SeqCst);
    });
}

fn store() {
    let file = data_dir().join("settings.json");
    let map = map().read().clone();
    let json = serde_json::to_string_pretty(&map).unwrap();
    if let Err(e) = std::fs::write(&file, json) {
        log::error!("Failed to write the settings file {file:?}: {e:?}");
    } else {
        log::info!("Settings saved to {file:?}");
    }
}
