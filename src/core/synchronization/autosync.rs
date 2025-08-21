// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::sync::atomic::{ AtomicBool, AtomicUsize, Ordering::Relaxed, Ordering::SeqCst };
use std::sync::Arc;
use std::borrow::Cow;
use itertools::Either;
use parking_lot::RwLock;

use crate::StabilizationManager;
use crate::stabilization::ComputeParams;
use super::PoseEstimator;
use super::SyncParams;

pub struct AutosyncProcess {
    frame_count: usize,
    scaled_fps: f64,
    org_fps: f64,
    fps_scale: Option<f64>,
    mode: String, // synchronize, guess_imu_orientation, estimate_rolling_shutter
    ranges_us: Vec<(i64, i64)>,
    scaled_ranges_us: Vec<(i64, i64)>,
    estimator: Arc<PoseEstimator>,
    total_read_frames: Arc<AtomicUsize>,
    total_detected_frames: Arc<AtomicUsize>,
    compute_params: Arc<RwLock<ComputeParams>>,
    cancel_flag: Arc<AtomicBool>,
    progress_cb: Option<Arc<Box<dyn Fn(f64, usize, usize) + Send + Sync + 'static>>>,
    finished_cb: Option<Arc<Box<dyn Fn(Either<Vec<(f64, f64, f64)>, Option<(String, f64)>>) + Send + Sync + 'static>>>,

    sync_params: SyncParams,

    thread_pool: rayon::ThreadPool,
}

impl AutosyncProcess {
    pub fn from_manager(stab: &StabilizationManager, timestamps_fract: &[f64], sync_params: SyncParams, mode: String, cancel_flag: Arc<AtomicBool>) -> Result<Self, ()> {
        let params = stab.params.read();
        let org_fps = params.fps;
        let scaled_fps = params.get_scaled_fps();
        let org_duration_ms = params.duration_ms;
        let fps_scale = params.fps_scale;
        let duration_ms = params.get_scaled_duration_ms();

        // 从一个名为sync_params的SyncParams结构体中，方便地提取出我们感兴趣的几个字段，
        // 并将它们绑定到新的局部变量上，同时忽略我们不关心的其他字段
        let SyncParams {
            search_size,
            mut time_per_syncpoint,
            every_nth_frame,
            ..
        } = sync_params;

        // 如果fps_scale存在，则将time_per_syncpoint乘以fps_scale
        if let Some(scale) = &fps_scale {
            time_per_syncpoint *= scale;
        }
        let frame_count = ((timestamps_fract.len() as f64 * (time_per_syncpoint / 1000.0) * org_fps).ceil() as usize).min(params.frame_count) / every_nth_frame as usize;

        drop(params); // 立即销毁变量，释放该变量所管理的资源

        if duration_ms < 10.0 || frame_count < 2 || time_per_syncpoint < 10.0 || search_size < 10.0 { return Err(()); }

        let mut ranges_us: Vec<(i64, i64)> = timestamps_fract.iter().map(|x| {
            let range = (
                ((x * org_duration_ms) - (time_per_syncpoint / 2.0)).max(0.0),
                ((x * org_duration_ms) + (time_per_syncpoint / 2.0)).min(org_duration_ms)
            );
            ((range.0 * 1000.0).round() as i64, (range.1 * 1000.0).round() as i64)
        }).collect();

        if mode == "synchronize" && !stab.gyro.read().has_motion() {
            // If no gyro data in file, analyze the entire video
            ranges_us.clear();
            ranges_us.push((0, (org_duration_ms * 1000.0).round() as i64));
        }

        let scaled_ranges_us = ranges_us.iter().map(|(f, t)| (
            (*f as f64 / fps_scale.unwrap_or(1.0)) as i64,
            (*t as f64 / fps_scale.unwrap_or(1.0)) as i64)
        ).collect();

        let estimator = stab.pose_estimator.clone();

        estimator.every_nth_frame.store(every_nth_frame.max(1) as u32, SeqCst);
        estimator.offset_method.store(sync_params.offset_method as u32, SeqCst);
        estimator.pose_method.store(sync_params.pose_method as u32, SeqCst);

        let mut comp_params = ComputeParams::from_manager(stab);
        comp_params.keyframes.clear();
        // Make sure we apply full correction for autosync
        comp_params.lens_correction_amount = 1.0;

        // 设置线程池的线程个数
        let thread_pool = rayon::ThreadPoolBuilder::new().num_threads(1)
            .thread_name(move |i| format!("Sync {}", i))
            .stack_size(10 * 1024 * 1024) // 10 MB
            .panic_handler(move |e| {
                if let Some(s) = e.downcast_ref::<&str>() {
                    log::error!("Sync thread panic! {}", s);
                } else if let Some(s) = e.downcast_ref::<String>() {
                    log::error!("Sync thread panic! {}", s);
                } else {
                    log::error!("Sync thread panic! {:?}", e);
                }
            })
            .build().unwrap();

        Ok(Self {
            frame_count,
            org_fps,
            scaled_fps,
            sync_params,
            mode,
            ranges_us,
            scaled_ranges_us,
            estimator,
            fps_scale,
            total_read_frames: Arc::new(AtomicUsize::new(1)), // Start with 1 to keep the loader active until `finished_feeding_frames` overrides it with final value
            total_detected_frames: Arc::new(AtomicUsize::new(0)),
            compute_params: Arc::new(RwLock::new(comp_params)),
            finished_cb: None,
            progress_cb: None,
            cancel_flag,
            thread_pool
        })
    }

    pub fn get_ranges(&self) -> Vec<(f64, f64)> {
        self.ranges_us.iter().map(|&v| (v.0 as f64 / 1000.0, v.1 as f64 / 1000.0)).collect()
    }

    pub fn feed_frame(&self, mut timestamp_us: i64, frame_no: usize, mut width: u32, height: u32, stride: usize, pixels: &[u8]) {
        // 将YUV图像转换为灰度图像，image::GrayImage以[x, y]和get_pixel(x, y)访问像素值的方式和OpenCV的at<>相反
        let img = PoseEstimator::yuv_to_gray(width, height, stride as u32, pixels).map(Arc::new);
        if width > stride as u32 {
            width = stride as u32;
        }

        let method = self.sync_params.of_method as u32;
        let estimator = self.estimator.clone();
        let total_detected_frames = self.total_detected_frames.clone();
        let total_read_frames = self.total_read_frames.clone();
        let progress_cb = self.progress_cb.clone();
        let frame_count = self.frame_count;
        let scaled_fps = self.scaled_fps;
        let org_fps = self.org_fps;
        let compute_params = self.compute_params.clone();
        let cancel_flag = self.cancel_flag.clone();
        if let Some(scale) = self.fps_scale {
            timestamp_us = (timestamp_us as f64 / scale) as i64;
        }

        {
            let compute_params = compute_params.read();
            let frame = crate::frame_at_timestamp(timestamp_us as f64 / 1000.0, compute_params.scaled_fps) as usize;
            timestamp_us += (compute_params.gyro.read().file_metadata.read().per_frame_time_offsets.get(frame).unwrap_or(&0.0) * 1000.0).round() as i64;
        }

        // iter是Rust所有集合（collection）都有的标准方法，它会创建一个迭代器，这个迭代器提供对集合元素的逐一地不可变引用访问
        // find是迭代器的一个核心方法，接收一个闭包或者函数作为参数，它会遍历迭代器中的每个元素，并将元素传递给这个闭包，闭包的返回值是一个布尔类型
        // 如果闭包返回true，find方法就会返回当前元素的引用；如果没有元素满足条件，则返回None，find括号内整个是一个闭包，||是闭包的参数列表
        // (*from..=*to): 这是Rust的范围语法，并且是闭区间版本，contains检查timestamp_us（闭包捕获的外部变量）是否在范围内
        // 整体作用是判断当前图像帧的时间戳是否在已知的同步范围内，如果在，则继续处理该帧
        if let Some(_current_range) = self.scaled_ranges_us.iter().find(|(from, to)| (*from..=*to).contains(&timestamp_us)) {
            self.total_read_frames.fetch_add(1, SeqCst);

            self.thread_pool.spawn(move || {
                if cancel_flag.load(Relaxed) {
                    total_detected_frames.fetch_add(1, SeqCst);
                    return;
                }
                if let Some(img) = img {
                    // 在当前帧的灰度图像上根据指定的光流方法检测特征点，构造FrameResult对象映射
                    estimator.detect_features(frame_no, timestamp_us, img, width, height, method);
                    total_detected_frames.fetch_add(1, SeqCst);

                    // 每隔7帧对检测到光流特征的视频帧进行帧间位姿估计，计算旋转矩阵，并转换为四元数和欧拉角存储
                    if frame_no % 7 == 0 {
                        estimator.process_detected_frames(org_fps, scaled_fps, &compute_params.read());
                        estimator.recalculate_gyro_data(org_fps, false);
                    }

                    if let Some(cb) = &progress_cb {
                        let d = total_detected_frames.load(SeqCst);
                        let t = total_read_frames.load(SeqCst).max(frame_count);
                        cb((d as f64 / t.max(1) as f64) * 0.58, d, t);
                    }
                } else {
                    log::warn!("Failed to get image {:?}", img);
                }
            });
        }
    }

    pub fn finished_feeding_frames(&self) {
        while self.total_detected_frames.load(SeqCst) < self.total_read_frames.load(SeqCst) - 1 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        let offset_method = self.sync_params.offset_method; // 估算偏移方法

        let progress_cb = self.progress_cb.clone();

        self.estimator.process_detected_frames(self.org_fps, self.scaled_fps, &self.compute_params.read());
        self.estimator.recalculate_gyro_data(self.org_fps, true);
        self.estimator.cache_optical_flow(if offset_method == 1 { 2 } else { 1 });
        self.estimator.cleanup();

        let mut scaled_ranges_us = Cow::Borrowed(&self.scaled_ranges_us);

        if self.mode == "synchronize" && !self.compute_params.read().gyro.read().has_motion() {
            // If no gyro data in file, set the computed optical flow as gyro data
            let compute_params = self.compute_params.write();
            let mut gyro = compute_params.gyro.write();

            gyro.file_metadata.set_raw_imu(self.estimator.estimated_gyro.read().values().cloned().collect::<Vec<_>>());
            gyro.apply_transforms();

            let timestamps_fract = [0.5];
            let time_per_syncpoint = 500.0;

            scaled_ranges_us = Cow::Owned(timestamps_fract.into_iter().map(|x| (
                (((x * gyro.duration_ms) - (time_per_syncpoint / 2.0)).max(0.0)              * 1000.0 / self.fps_scale.unwrap_or(1.0)).round() as i64,
                (((x * gyro.duration_ms) + (time_per_syncpoint / 2.0)).min(gyro.duration_ms) * 1000.0 / self.fps_scale.unwrap_or(1.0)).round() as i64
            )).collect());
        }

        if let Some(cb) = &progress_cb {
            let d = self.total_detected_frames.load(SeqCst);
            let t = self.total_read_frames.load(SeqCst);
            cb(0.6, d, t);
        }

        let check_negative = self.sync_params.initial_offset_inv && self.sync_params.initial_offset.abs() > 1.0;

        let for_negative = AtomicBool::new(false);

        let progress_cb2 = |mut progress| {
            if let Some(cb) = &progress_cb {
                let d = self.total_detected_frames.load(SeqCst);
                let t = self.total_read_frames.load(SeqCst);
                if check_negative {
                    progress += if for_negative.load(SeqCst) { 1.0 } else { 0.0 };
                    progress /= 2.0;
                }
                cb(0.6 + (progress * 0.4), d, t);
            }
        };

        if let Some(cb) = &self.finished_cb {
            if self.mode == "estimate_rolling_shutter" {
                use super::find_offset::visual_features::find_offsets;
                cb(Either::Left(find_offsets(&self.estimator, &scaled_ranges_us, &self.sync_params, &self.compute_params.read(), true, progress_cb2, self.cancel_flag.clone())));
            } else if self.mode == "guess_imu_orientation" {
                use super::find_offset::rs_sync::FindOffsetsRssync;
                let guessed = FindOffsetsRssync::new(&scaled_ranges_us, self.estimator.sync_results.clone(), &self.sync_params, &self.compute_params.read(), progress_cb2, self.cancel_flag.clone()).guess_orient();
                if !self.cancel_flag.load(SeqCst) {
                    cb(Either::Right(guessed));
                }
            } else {
                // 估计时间偏移
                let offsets = self.estimator.find_offsets(&scaled_ranges_us, &self.sync_params, &self.compute_params.read(), progress_cb2, self.cancel_flag.clone());
                if check_negative {
                    for_negative.store(true, SeqCst);
                    // Try also negative rough offset
                    let mut sync_params = self.sync_params.clone();
                    sync_params.initial_offset = -sync_params.initial_offset;
                    let offsets2 = self.estimator.find_offsets(&scaled_ranges_us, &sync_params, &self.compute_params.read(), progress_cb2, self.cancel_flag.clone());
                    if offsets2.len() > offsets.len() {
                        cb(Either::Left(offsets2));
                    } else if offsets2.len() == offsets.len() {
                        let sum1: f64 = offsets.iter().map(|(_, _, cost)| *cost).sum();
                        let sum2: f64 = offsets2.iter().map(|(_, _, cost)| *cost).sum();
                        if sum1 < sum2 {
                            cb(Either::Left(offsets));
                        } else {
                            cb(Either::Left(offsets2));
                        }
                    }
                } else {
                    cb(Either::Left(offsets));
                }
            }
        }
        if let Some(cb) = &self.progress_cb {
            let len = self.total_detected_frames.load(SeqCst);
            cb(1.0, len, len);
        }
    }

    pub fn on_progress<F>(&mut self, cb: F) where F: Fn(f64, usize, usize) + Send + Sync + 'static {
        self.progress_cb = Some(Arc::new(Box::new(cb)));
    }
    pub fn on_finished<F>(&mut self, cb: F) where F:  Fn(Either<Vec<(f64, f64, f64)>, Option<(String, f64)>>) + Send + Sync + 'static {
        self.finished_cb = Some(Arc::new(Box::new(cb)));
    }
}
