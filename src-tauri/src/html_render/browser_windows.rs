use std::{
    ffi::{OsStr, OsString},
    io,
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    time::Duration,
};

use windows::{
    core::{PCWSTR, PWSTR},
    Win32::{
        Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE},
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
                JobObjectExtendedLimitInformation, QueryInformationJobObject,
                SetInformationJobObject, TerminateJobObject,
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Pipes::CreatePipe,
            Threading::{
                CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess,
                InitializeProcThreadAttributeList, ResumeThread, TerminateProcess,
                UpdateProcThreadAttribute, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED,
                EXTENDED_STARTUPINFO_PRESENT, INFINITE, LPPROC_THREAD_ATTRIBUTE_LIST,
                PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES,
                STARTUPINFOEXW,
            },
        },
    },
};

fn handle(raw: &OwnedHandle) -> HANDLE {
    HANDLE(raw.as_raw_handle())
}

fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write) = (HANDLE::default(), HANDLE::default());
    unsafe { CreatePipe(&mut read, &mut write, None, 0) }.map_err(io::Error::other)?;
    Ok(unsafe {
        (
            OwnedHandle::from_raw_handle(read.0),
            OwnedHandle::from_raw_handle(write.0),
        )
    })
}

fn inheritable(source: &OwnedHandle) -> io::Result<OwnedHandle> {
    let mut duplicate = HANDLE::default();
    unsafe {
        let process = GetCurrentProcess();
        DuplicateHandle(
            process,
            handle(source),
            process,
            &mut duplicate,
            0,
            true,
            DUPLICATE_SAME_ACCESS,
        )
        .map_err(io::Error::other)?;
        Ok(OwnedHandle::from_raw_handle(duplicate.0))
    }
}

struct Attributes {
    raw: LPPROC_THREAD_ATTRIBUTE_LIST,
    _storage: Vec<usize>,
}

impl Attributes {
    fn new(handles: &[HANDLE]) -> io::Result<Self> {
        let mut size = 0;
        let _ = unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut size) };
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut storage = vec![0usize; (size + size_of::<usize>() - 1) / size_of::<usize>()];
        let raw = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast());
        unsafe { InitializeProcThreadAttributeList(Some(raw), 1, None, &mut size) }
            .map_err(io::Error::other)?;
        let attributes = Self {
            raw,
            _storage: storage,
        };
        unsafe {
            UpdateProcThreadAttribute(
                raw,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_ptr().cast()),
                std::mem::size_of_val(handles),
                None,
                None,
            )
        }
        .map_err(io::Error::other)?;
        Ok(attributes)
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.raw);
        }
    }
}

fn append_quoted(command: &mut Vec<u16>, arg: &OsStr) -> io::Result<()> {
    command.push(b'"' as u16);
    let mut slashes = 0;
    for ch in arg.encode_wide() {
        match ch {
            0 => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "NUL in browser argument",
                ))
            }
            92 => slashes += 1,
            34 => {
                command.extend(std::iter::repeat_n(92, slashes * 2 + 1));
                command.push(ch);
                slashes = 0;
            }
            _ => {
                command.extend(std::iter::repeat_n(92, slashes));
                command.push(ch);
                slashes = 0;
            }
        }
    }
    command.extend(std::iter::repeat_n(92, slashes * 2));
    command.push(b'"' as u16);
    Ok(())
}

pub(super) struct BrowserChild {
    job: OwnedHandle,
    process: OwnedHandle,
}

pub(super) fn spawn_browser(
    exe: &Path,
    args: &[OsString],
) -> io::Result<(
    BrowserChild,
    tokio::fs::File,
    tokio::fs::File,
    tokio::fs::File,
)> {
    let (child_read, parent_write) = pipe()?;
    let (parent_read, child_write) = pipe()?;
    let (parent_stderr, child_stderr) = pipe()?;
    let child_read = inheritable(&child_read)?;
    let child_write = inheritable(&child_write)?;
    let child_stderr = inheritable(&child_stderr)?;
    let null: OwnedHandle = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("NUL")?
        .into();
    let null_read = inheritable(&null)?;
    let null_write = inheritable(&null)?;
    let handles = [
        handle(&null_read),
        handle(&null_write),
        handle(&child_stderr),
        handle(&child_read),
        handle(&child_write),
    ];

    let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(io::Error::other)?;
    let job = unsafe { OwnedHandle::from_raw_handle(job.0) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    unsafe {
        SetInformationJobObject(
            handle(&job),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    }
    .map_err(io::Error::other)?;

    // The explicit list prevents unrelated inheritable handles reaching the browser.
    let attributes = Attributes::new(&handles)?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = handles[0];
    startup.StartupInfo.hStdOutput = handles[1];
    startup.StartupInfo.hStdError = handles[2];
    // The CRT inherits fd 3/4 through this packed table, as with Node's pipe stdio.
    let mut crt = [0u8; size_of::<u32>() + 5 + 5 * size_of::<usize>()];
    crt[..4].copy_from_slice(&5u32.to_ne_bytes());
    crt[4..9].copy_from_slice(&[0x41, 0x41, 0x09, 0x09, 0x09]);
    for (index, entry) in handles.iter().enumerate() {
        let start = 9 + index * size_of::<usize>();
        crt[start..start + size_of::<usize>()].copy_from_slice(&(entry.0 as usize).to_ne_bytes());
    }
    startup.StartupInfo.cbReserved2 = crt.len() as u16;
    startup.StartupInfo.lpReserved2 = crt.as_mut_ptr();
    startup.lpAttributeList = attributes.raw;
    let mut command = Vec::new();
    append_quoted(&mut command, exe.as_os_str())?;
    for arg in args {
        command.push(b' ' as u16);
        append_quoted(&mut command, arg)?;
    }
    command.push(0);
    let mut executable: Vec<u16> = exe.as_os_str().encode_wide().collect();
    executable.push(0);
    let mut process = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessW(
            PCWSTR(executable.as_ptr()),
            Some(PWSTR(command.as_mut_ptr())),
            None,
            None,
            true,
            CREATE_NO_WINDOW | CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
            None,
            PCWSTR::null(),
            &startup.StartupInfo,
            &mut process,
        )
    }
    .map_err(io::Error::other)?;
    drop(attributes);
    drop(child_read);
    drop(child_write);
    drop(child_stderr);
    drop(null_read);
    drop(null_write);
    let process_handle = unsafe { OwnedHandle::from_raw_handle(process.hProcess.0) };
    let thread = unsafe { OwnedHandle::from_raw_handle(process.hThread.0) };
    if let Err(error) = unsafe { AssignProcessToJobObject(handle(&job), handle(&process_handle)) } {
        unsafe {
            let _ = TerminateProcess(handle(&process_handle), 1);
            WaitForSingleObject(handle(&process_handle), INFINITE);
        }
        return Err(io::Error::other(error));
    }
    if unsafe { ResumeThread(handle(&thread)) } == u32::MAX {
        let error = io::Error::last_os_error();
        unsafe {
            let _ = TerminateJobObject(handle(&job), 1);
            WaitForSingleObject(handle(&process_handle), INFINITE);
        }
        return Err(error);
    }
    let reader = tokio::fs::File::from_std(std::fs::File::from(parent_read));
    let writer = tokio::fs::File::from_std(std::fs::File::from(parent_write));
    let stderr = tokio::fs::File::from_std(std::fs::File::from(parent_stderr));
    Ok((
        BrowserChild {
            job,
            process: process_handle,
        },
        reader,
        writer,
        stderr,
    ))
}

pub(super) async fn stop_browser(browser: BrowserChild) -> io::Result<()> {
    unsafe { TerminateJobObject(handle(&browser.job), 1) }.map_err(io::Error::other)?;
    loop {
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        unsafe {
            QueryInformationJobObject(
                Some(handle(&browser.job)),
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                None,
            )
        }
        .map_err(io::Error::other)?;
        if info.ActiveProcesses == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // The job is empty before the profile is deleted; the process handle stays owned until here.
    drop(browser.process);
    Ok(())
}
