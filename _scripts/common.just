set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]
# 路径分隔符定义
PathSep := if os() == "windows" { ";" } else { ":" }

ProjectVersion := "1.6.1"
# 项目根目录检测，justfile_directory()返回当前justfile所在的目录，path_exists()检查文件或路径是否存在
ProjectDir := if path_exists(justfile_directory() / "Cargo.toml") == "true" { justfile_directory()
       } else if path_exists(justfile_directory() / ".." / "Cargo.toml") == "true" { justfile_directory() / "../"
       } else { justfile_directory() }
# 设置外部依赖目录
ExtDir := ProjectDir / "ext"
# arch()返回当前系统的架构类型（如x86_64, aarch64等），env读取环境变量，带默认值
# 先检查FORCE_ARCH环境变量（允许强制指定架构），否则使用系统自动检测的架构
Arch := env("FORCE_ARCH", arch())

# ――――――――――――――――――――――――――――――――――――――――― Qt ――――――――――――――――――――――――――――――――――――――――――
# env_var_or_default函数用于获取环境变量的值，如果不存在则使用默认值
QtVersion := env_var_or_default("FORCE_QT_VERSION", if os() == "windows" { "6.7.3" } else { if Arch == "aarch64" { "6.7.3" } else { "6.4.3" } })
# 根据操作系统和架构类型设置对应的Qt版本
QtArch := if os() + Arch == "windowsx86_64"  { "msvc2019_64"
   } else if os() + Arch == "windowsaarch64" { "msvc2019_arm64"
   } else if os() == "macos"   { "macos"
   } else if Arch == "aarch64" { "gcc_arm64"
   } else                      { "gcc_64" }
# 设置Qt安装路径
QtPath := ProjectDir / "ext/" / QtVersion / QtArch / "bin"
# 导出qmake可执行文件路径的环境变量
export QMAKE := env_var_or_default("QMAKE", if path_exists(QtPath / "qmake.bat") == "true" { QtPath / "qmake.bat" } else { QtPath / "qmake" })
# ――――――――――――――――――――――――――――――――――――――――― Qt ――――――――――――――――――――――――――――――――――――――――――

# ――――――――――――――――――――――――――――――――――――――― FFmpeg ――――――――――――――――――――――――――――――――――――――――
# 根据操作系统和架构类型设置FFmpeg版本
FFmpeg := if os() == "windows"               { "ffmpeg-7.1-windows-desktop-clang-gpl-lite"
   } else if os() + Arch == "macosaarch64"   { "ffmpeg-arm64"
   } else if os() + Arch == "macosx86_64"    { "ffmpeg-x86_64"
   } else                                    { "ffmpeg-7.1-linux-clang-gpl-lite" }
# 导出FFmpeg安装目录路径的环境变量
export FFMPEG_DIR := ProjectDir / "ext" / FFmpeg
# 导出FFmpeg架构类型的环境变量
export FFMPEG_ARCH := if Arch == "aarch64" { "arm64" } else { if os() == "windows" { "x64" } else { "amd64" } }
# ――――――――――――――――――――――――――――――――――――――― FFmpeg ――――――――――――――――――――――――――――――――――――――――

# ――――――――――――――――――――――――――――――――――――――― OpenCV ――――――――――――――――――――――――――――――――――――――――
# 根据系统和架构设置vcpkg的版本
VcpkgArch := if os() + Arch == "windowsx86_64"  { "x64-windows"
      } else if os() + Arch == "windowsaarch64" { "arm64-windows"
      } else if os() + Arch == "macosaarch64"   { "arm64-osx"
      } else if os() + Arch == "macosx86_64"    { "x64-osx"
      } else if Arch == "aarch64"               { "arm64-linux"
      } else                                    { "x64-linux" }
# 设置OpenCV的链接库路径和头文件路径
export OPENCV_LINK_LIBS := "opencv_core4,opencv_calib3d4,opencv_features2d4,opencv_imgproc4,opencv_video4,opencv_flann4,opencv_imgcodecs4,opencv_objdetect4,opencv_stitching4" + if os() == "linux" { if Arch == "aarch64" { ",tegra_hal,png" } else { ",png" } } else { "" }
export OPENCV_LINK_PATHS := ProjectDir / "ext/vcpkg/installed" / VcpkgArch / "lib," + ProjectDir / "ext/vcpkg/installed" / VcpkgArch / "lib/manual-link/opencv4_thirdparty"
export OPENCV_INCLUDE_PATHS := ProjectDir / "ext/vcpkg/installed" / VcpkgArch / "include/opencv4"
OpenCVPath := ProjectDir / "ext/vcpkg/installed" / VcpkgArch
# ――――――――――――――――――――――――――――――――――――――― OpenCV ――――――――――――――――――――――――――――――――――――――――

# ――――――――――――――――――――――――――――――――――――――― MacOS ―――――――――――――――――――――――――――――――――――――――――
export DYLD_FALLBACK_LIBRARY_PATH := if os() == "macos" { if path_exists(`xcode-select --print-path` + "/Toolchains/XcodeDefault.xctoolchain/usr/lib/") == "true" { `xcode-select --print-path` + "/Toolchains/XcodeDefault.xctoolchain/usr/lib/" } else { `xcode-select --print-path` + "/usr/lib/" } } else { "" }
export MACOSX_DEPLOYMENT_TARGET := "10.15"
# ――――――――――――――――――――――――――――――――――――――― MacOS ―――――――――――――――――――――――――――――――――――――――――

# ――――――――――――――――――――――――――――――――――――――― Clang ―――――――――――――――――――――――――――――――――――――――――
# 检测LLVM配置工具是否存在，适用于Linux系统，> /dev/null将which的标准输出（stdout）重定向到/dev/null（即丢弃输出）。
#这样做的目的是只关心which的退出状态码，而不需要看到路径，&&表示前一条命令成功（退出码 0）时才执行后续命令
LlvmConfigExists := if os() == "linux" { `which llvm-config > /dev/null && echo true || echo false` } else { "false" }
export LIBCLANG_PATH := if os() == "macos" {
   DYLD_FALLBACK_LIBRARY_PATH
} else {
   if path_exists(ExtDir / "llvm/bin") == "true" {
      ExtDir / "llvm/bin"
   } else {
      env_var_or_default("LIBCLANG_PATH", if "{{LlvmConfigExists}}" == "true" {
         `llvm-config --libdir`
      } else {
         if path_exists("/usr/lib/llvm-13/lib/") == "true" { "/usr/lib/llvm-13/lib/" } else { "" }
      })
   }
}
# 设置LLVM的库路径
LLVMPath := LIBCLANG_PATH
# ――――――――――――――――――――――――――――――――――――――― Clang ―――――――――――――――――――――――――――――――――――――――――


export LD_LIBRARY_PATH := ProjectDir / "target/release:" + QtPath / "../lib:" + FFMPEG_DIR + "/lib:" + FFMPEG_DIR + "/lib/" + FFMPEG_ARCH

export PATH := QtPath + PathSep + FFMPEG_DIR + "/bin" + PathSep + FFMPEG_DIR + "/bin/" + FFMPEG_ARCH + PathSep + OpenCVPath + "/bin" + PathSep + LLVMPath + PathSep + env_var('PATH')

# export VULKAN_SDK := "D:/Programy/VulkanSDK/1.3.236.0"
