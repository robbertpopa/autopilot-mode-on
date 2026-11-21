//! Only the three stdio pipe handles are inherited. The job stays with the supervisor.

use std::{
    ffi::OsStr,
    io::{self, pipe},
    mem::{size_of, size_of_val},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::ExitStatusExt,
    },
    path::Path,
    process::ExitStatus,
    ptr::{null, null_mut},
    thread::JoinHandle,
};

use windows_sys::Win32::{
    Foundation::{HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{
        JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
        Threading::{
            CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
            InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
            STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};

use crate::{CLEANUP_TIMEOUT, CommandSpec, Pipes, feed, joined};

pub(crate) struct Active {
    job: Option<OwnedHandle>,
    process: OwnedHandle,
    input: Option<JoinHandle<io::Result<()>>>,
}

impl Active {
    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        // SAFETY: The process handle stays open across both kernel queries.
        unsafe {
            match WaitForSingleObject(self.process.as_raw_handle(), 0) {
                WAIT_TIMEOUT => Ok(None),
                WAIT_OBJECT_0 => {
                    let mut code = 0;
                    success(GetExitCodeProcess(
                        self.process.as_raw_handle(),
                        &raw mut code,
                    ))?;
                    Ok(Some(ExitStatus::from_raw(code)))
                }
                _ => Err(io::Error::last_os_error()),
            }
        }
    }

    pub(crate) fn shutdown(&mut self) -> io::Result<()> {
        drop(self.job.take());
        let timeout = u32::try_from(CLEANUP_TIMEOUT.as_millis()).map_err(io::Error::other)?;
        // SAFETY: The process handle remains owned by self until the wait finishes.
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), timeout) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "job did not finish cleanup",
                ));
            }
            _ => return Err(io::Error::last_os_error()),
        }
        if let Some(input) = self.input.take() {
            joined(input)?;
        }
        Ok(())
    }
}

pub(crate) fn start(_: &Path, spec: &CommandSpec, input: &[u8]) -> io::Result<(Active, Pipes)> {
    let (child_stdin, stdin) = pipe()?;
    let (stdout, child_stdout) = pipe()?;
    let (stderr, child_stderr) = pipe()?;
    let handles = [
        child_stdin.as_raw_handle(),
        child_stdout.as_raw_handle(),
        child_stderr.as_raw_handle(),
    ];
    let job = new_job()?;
    let jobs = [job.as_raw_handle()];

    // SAFETY: The pipe endpoints are valid and the parent ends remain non-inheritable.
    for handle in handles {
        success(unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) })?;
    }
    let mut attributes = Attributes::new()?;
    attributes.add(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &handles)?;
    attributes.add(PROC_THREAD_ATTRIBUTE_JOB_LIST, &jobs)?;

    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb =
        u32::try_from(size_of::<STARTUPINFOEXW>()).map_err(io::Error::other)?;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    [
        startup.StartupInfo.hStdInput,
        startup.StartupInfo.hStdOutput,
        startup.StartupInfo.hStdError,
    ] = handles;
    startup.lpAttributeList = attributes.as_ptr();

    let application: Vec<u16> = wide(spec.program.as_os_str())?
        .into_iter()
        .chain([0])
        .collect();
    let mut command = command_line(spec)?;
    let environment = environment_block(spec)?;
    let directory: Option<Vec<u16>> = match &spec.cwd {
        Some(dir) => Some(wide(dir.as_os_str())?.into_iter().chain([0]).collect()),
        None => None,
    };
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: All string buffers, attributes, handles and output storage stay valid through the call.
    // Assigning the job during creation avoids leaving a suspended orphan if the supervisor exits.
    success(unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            environment.as_ptr().cast(),
            directory.as_ref().map_or(null(), Vec::as_ptr),
            &raw const startup.StartupInfo,
            &raw mut info,
        )
    })?;
    // SAFETY: A successful CreateProcessW transfers both handles to this caller.
    let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
    // SAFETY: The primary thread handle is distinct and this scope is its sole owner.
    let thread = unsafe { OwnedHandle::from_raw_handle(info.hThread) };
    drop(thread);
    drop((child_stdin, child_stdout, child_stderr));
    drop(attributes);

    let mut active = Active {
        job: Some(job),
        process,
        input: None,
    };
    active.input = Some(feed(stdin, input.to_vec())?);
    Ok((
        active,
        Pipes {
            stdout: Box::new(stdout),
            stderr: Box::new(stderr),
        },
    ))
}

fn new_job() -> io::Result<OwnedHandle> {
    // SAFETY: Null arguments request a new unnamed job with a non-inheritable handle.
    let raw = unsafe { CreateJobObjectW(null(), null()) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateJobObjectW returned a valid handle whose ownership was transferred to us.
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut data = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    data.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let size = u32::try_from(size_of_val(&data)).map_err(io::Error::other)?;
    // SAFETY: The size and information class match the live structure passed to the kernel.
    success(unsafe {
        SetInformationJobObject(
            raw,
            JobObjectExtendedLimitInformation,
            (&raw const data).cast(),
            size,
        )
    })?;
    Ok(job)
}

struct Attributes(Vec<usize>);

impl Attributes {
    fn new() -> io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: The null probe writes the required size to the writable length argument.
        unsafe { InitializeProcThreadAttributeList(null_mut(), 2, 0, &raw mut bytes) };
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut storage = vec![0_usize; bytes.div_ceil(size_of::<usize>())];
        // SAFETY: The backing allocation is pointer-aligned and at least the requested size.
        success(unsafe {
            InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), 2, 0, &raw mut bytes)
        })?;
        Ok(Self(storage))
    }

    fn as_ptr(&mut self) -> windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST {
        self.0.as_mut_ptr().cast()
    }

    fn add(&mut self, attribute: u32, handles: &[HANDLE]) -> io::Result<()> {
        // SAFETY: In start(), both handle arrays outlive the attribute list and child creation.
        success(unsafe {
            UpdateProcThreadAttribute(
                self.as_ptr(),
                0,
                attribute as usize,
                handles.as_ptr().cast(),
                size_of_val(handles),
                null_mut(),
                null(),
            )
        })
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: Only an initialized attribute list is wrapped by Attributes.
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
    }
}

fn success(code: i32) -> io::Result<()> {
    if code == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let encoded: Vec<_> = value.encode_wide().collect();
    if encoded.contains(&0) {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "embedded NUL in process arguments",
        ))
    } else {
        Ok(encoded)
    }
}

fn command_line(spec: &CommandSpec) -> io::Result<Vec<u16>> {
    let mut result = Vec::new();
    quote(&mut result, &wide(spec.program.as_os_str())?);
    for argument in &spec.args {
        result.push(u16::from(b' '));
        quote(&mut result, &wide(argument)?);
    }
    result.push(0);
    Ok(result)
}

fn quote(result: &mut Vec<u16>, argument: &[u16]) {
    const SLASH: u16 = b'\\' as u16;
    const QUOTE: u16 = b'"' as u16;
    result.push(QUOTE);
    let mut slashes = 0;
    for &unit in argument {
        match unit {
            SLASH => slashes += 1,
            QUOTE => {
                result.extend(std::iter::repeat_n(SLASH, slashes * 2 + 1));
                result.push(QUOTE);
                slashes = 0;
            }
            _ => {
                result.extend(std::iter::repeat_n(SLASH, slashes));
                result.push(unit);
                slashes = 0;
            }
        }
    }
    result.extend(std::iter::repeat_n(SLASH, slashes * 2));
    result.push(QUOTE);
}

fn environment_block(spec: &CommandSpec) -> io::Result<Vec<u16>> {
    let mut block = Vec::new();
    let mut pairs = spec.env.iter().collect::<Vec<_>>();
    pairs.sort_by_cached_key(|(key, _)| key.to_string_lossy().to_ascii_uppercase());
    let mut previous = None;
    for (key, value) in pairs {
        let key = wide(key)?;
        if key.is_empty() || key.contains(&u16::from(b'=')) || previous.as_ref() == Some(&key) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid or duplicate environment name",
            ));
        }
        block.extend(&key);
        block.push(u16::from(b'='));
        block.extend(wide(value)?);
        block.push(0);
        previous = Some(key);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}
