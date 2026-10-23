//! Durable file replacement shared by the desktop hosts.
use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

const WRITE_RETRY_ATTEMPTS: usize = 3;

#[cfg(test)]
thread_local! {
    static FAIL_ATOMIC_REPLACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn fail_atomic_replace_for_current_test(enabled: bool) {
    FAIL_ATOMIC_REPLACE.set(enabled);
}

pub(crate) fn temporary_write_path(path: &Path) -> PathBuf {
    path.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            ".{}.tmp.{}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("conversation"),
            uuid::Uuid::new_v4()
        ))
}

pub(crate) fn atomic_write(path: &Path, content: &str, label: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{label} path has no parent"))?;
    fs::create_dir_all(parent).map_err(|e| format!("create {label} dir: {e}"))?;

    for attempt in 0..WRITE_RETRY_ATTEMPTS {
        let tmp_path = temporary_write_path(path);

        // 直接 rename 覆盖:Windows/Unix 的 fs::rename 都会原子替换已存在目标。
        // 绝不"先 remove 再 rename"——那会制造"目标文件中途消失"的窗口:一旦紧接的
        // rename 失败,index.json 就没了,下次读到空索引会把其余对话文件全部孤立(数据看似丢失)。
        // 瞬时失败(锁 / 杀软占用)交给下面的外层重试循环 sleep 后重试整次写,期间旧文件始终保留。
        //
        // rename 之前必须 sync_all():否则数据还在页缓存里、rename 的元数据却可能先落盘,
        // 断电后 conv_x.json 会变成 0 字节或被截断——load_conversation 硬报错,而列表扫描
        // 又会静默跳过它,用户看到的就是"这条对话没了"。
        let write_result = (|| {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp_path)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            drop(file);
            #[cfg(test)]
            if FAIL_ATOMIC_REPLACE.get() {
                return Err(std::io::Error::other("injected atomic replace failure"));
            }
            fs::rename(&tmp_path, path)
        })();

        match write_result {
            Ok(()) => return Ok(()),
            Err(e) if attempt + 1 < WRITE_RETRY_ATTEMPTS => {
                let _ = fs::remove_file(&tmp_path);
                thread::sleep(Duration::from_millis(20 * (attempt as u64 + 1)));
                if e.kind() == ErrorKind::NotFound {
                    fs::create_dir_all(parent).map_err(|e| format!("create {label} dir: {e}"))?;
                }
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp_path);
                return Err(format!("write {label} file: {e}"));
            }
        }
    }

    Err(format!("write {label} file failed"))
}
