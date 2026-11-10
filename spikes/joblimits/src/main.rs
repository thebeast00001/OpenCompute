//! Spike 3: can the agent cap and reliably kill a task's processes on Windows?
//!
//! Questions:
//! 1. Does a Job Object CPU cap hold, and can it be changed while the task
//!    runs? (This is how a task yields when the owner starts using the PC.)
//! 2. Does a Job Object memory cap hold, and how does the task see it?
//! 3. Does closing the job kill the task and everything it started?
//!
//! The same binary is its own test workload: `spike-joblimits worker <mode>`.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("worker") {
        worker::run(&args[2..]);
        return;
    }
    #[cfg(windows)]
    parent::run();
    #[cfg(not(windows))]
    eprintln!("This spike covers Windows Job Objects. The Linux half (cgroups v2) is still to do.");
}

/// The workloads the parent puts inside a job.
mod worker {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    pub fn run(args: &[String]) {
        // Block until the parent says go, so the parent can place this
        // process in a job before it does anything. With no stdin this
        // returns at once.
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);

        let num = |i: usize| -> u64 { args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0) };
        match args.first().map(String::as_str) {
            Some("cpu") => burn_cpu(num(1) as usize, num(2)),
            Some("mem") => grab_memory(num(1), num(2)),
            Some("spawn") => spawn_grandchild(),
            Some("sleep") => std::thread::sleep(Duration::from_secs(num(1))),
            _ => std::process::exit(2),
        }
    }

    fn burn_cpu(threads: usize, seconds: u64) {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                std::thread::spawn(move || {
                    let mut x = 0u64;
                    while Instant::now() < deadline {
                        for _ in 0..100_000 {
                            x = std::hint::black_box(x.wrapping_mul(31).wrapping_add(7));
                        }
                    }
                })
            })
            .collect();
        for h in handles {
            let _ = h.join();
        }
    }

    /// Allocate and touch memory in steps, reporting progress, until the
    /// target is reached or an allocation fails.
    fn grab_memory(step_mib: u64, max_mib: u64) {
        let step = (step_mib as usize) << 20;
        let mut blocks: Vec<Vec<u8>> = Vec::new();
        let mut total = 0u64;
        let out = std::io::stdout();
        while total < max_mib {
            let mut block: Vec<u8> = Vec::new();
            if block.try_reserve_exact(step).is_err() {
                let _ = writeln!(out.lock(), "failed {total}");
                let _ = out.lock().flush();
                std::thread::sleep(Duration::from_secs(60));
                return;
            }
            block.resize(step, 1); // touch every page
            blocks.push(block);
            total += step_mib;
            let _ = writeln!(out.lock(), "allocated {total}");
            let _ = out.lock().flush();
        }
        let _ = writeln!(out.lock(), "done {total}");
        let _ = out.lock().flush();
        std::thread::sleep(Duration::from_secs(60));
    }

    // The grandchild is deliberately left running: the test is whether
    // closing the job kills it.
    #[allow(clippy::zombie_processes)]
    fn spawn_grandchild() {
        let exe = std::env::current_exe().expect("own path");
        let child = Command::new(exe)
            .args(["worker", "sleep", "120"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn grandchild");
        println!("grandchild {}", child.id());
        let _ = std::io::stdout().flush();
        std::thread::sleep(Duration::from_secs(120));
    }
}

#[cfg(windows)]
mod parent {
    use std::ffi::c_void;
    use std::io::{BufRead, BufReader, Write};
    use std::os::windows::io::AsRawHandle;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_CPU_RATE_CONTROL,
        JOB_OBJECT_CPU_RATE_CONTROL_ENABLE, JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
        JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_CPU_RATE_CONTROL_INFORMATION,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicAccountingInformation,
        JobObjectCpuRateControlInformation, JobObjectExtendedLimitInformation,
        QueryInformationJobObject, SetInformationJobObject,
    };
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::core::PCWSTR;

    const STILL_ACTIVE: u32 = 259;

    /// A Job Object: a kernel container for a task's whole process tree.
    struct Job(HANDLE);

    impl Job {
        /// New job that kills every process in it when the handle closes,
        /// optionally with a cap on total committed memory.
        fn new(memory_limit: Option<usize>) -> Self {
            // SAFETY: the struct is fully initialised and its size is passed.
            unsafe {
                let job = CreateJobObjectW(None, PCWSTR::null()).expect("CreateJobObjectW");
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if let Some(bytes) = memory_limit {
                    info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
                    info.JobMemoryLimit = bytes;
                }
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&raw const info).cast::<c_void>(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
                .expect("set extended limits");
                Job(job)
            }
        }

        /// Cap the job at a share of the whole machine's CPU. `None` asks
        /// Windows to switch rate control off. Can be called at any time
        /// while the job runs.
        fn set_cpu_cap(&self, percent_of_machine: Option<f64>) -> windows::core::Result<()> {
            let mut info = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION::default();
            match percent_of_machine {
                Some(p) => {
                    info.ControlFlags =
                        JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP;
                    // Units are hundredths of a percent; the valid range is 1..=10000.
                    info.Anonymous.CpuRate = ((p * 100.0) as u32).clamp(1, 10_000);
                }
                None => info.ControlFlags = JOB_OBJECT_CPU_RATE_CONTROL(0),
            }
            // SAFETY: as above.
            unsafe {
                SetInformationJobObject(
                    self.0,
                    JobObjectCpuRateControlInformation,
                    (&raw const info).cast::<c_void>(),
                    size_of::<JOBOBJECT_CPU_RATE_CONTROL_INFORMATION>() as u32,
                )
            }
        }

        fn assign(&self, child: &Child) {
            // SAFETY: the child handle is valid for the life of `child`.
            unsafe {
                AssignProcessToJobObject(self.0, HANDLE(child.as_raw_handle()))
                    .expect("AssignProcessToJobObject");
            }
        }

        /// Total CPU time used by every process that has been in the job,
        /// and how many are alive now.
        fn accounting(&self) -> (Duration, u32) {
            let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            // SAFETY: the buffer is the declared size.
            unsafe {
                QueryInformationJobObject(
                    Some(self.0),
                    JobObjectBasicAccountingInformation,
                    (&raw mut info).cast::<c_void>(),
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    None,
                )
                .expect("query accounting");
            }
            let ticks = (info.TotalUserTime + info.TotalKernelTime) as u64; // 100 ns units
            (Duration::from_nanos(ticks * 100), info.ActiveProcesses)
        }

        fn peak_memory(&self) -> usize {
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            // SAFETY: the buffer is the declared size.
            unsafe {
                QueryInformationJobObject(
                    Some(self.0),
                    JobObjectExtendedLimitInformation,
                    (&raw mut info).cast::<c_void>(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                    None,
                )
                .expect("query limits");
            }
            info.PeakJobMemoryUsed
        }

        fn close(self) {
            // SAFETY: the handle is owned by this value and not used again.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// Start a worker that waits on stdin until `go` is called.
    fn start_worker(args: &[&str]) -> Child {
        let exe = std::env::current_exe().expect("own path");
        Command::new(exe)
            .arg("worker")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn worker")
    }

    fn go(child: &mut Child) {
        let stdin = child.stdin.as_mut().expect("worker stdin");
        stdin.write_all(b"go\n").expect("signal worker");
        stdin.flush().expect("flush");
    }

    fn is_alive(pid: u32) -> Option<HANDLE> {
        // SAFETY: plain query; the handle is returned to the caller to poll.
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok() }
    }

    fn still_running(handle: HANDLE) -> bool {
        let mut code = 0u32;
        // SAFETY: handle came from OpenProcess above.
        unsafe { GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE }
    }

    pub fn run() {
        let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
        println!("Logical CPUs: {cpus}\n");
        cpu_cap_test(cpus);
        memory_cap_test();
        kill_on_close_test();
    }

    fn cpu_cap_test(cpus: usize) {
        println!("== 1. CPU cap, changed while the task runs ==");
        let setup = Instant::now();
        let job = Job::new(None);
        let threads = cpus.to_string();
        let mut child = start_worker(&["cpu", &threads, "30"]);
        job.assign(&child);
        println!(
            "Create job + start process + assign: {:.1} ms",
            setup.elapsed().as_secs_f64() * 1000.0
        );
        go(&mut child);
        std::thread::sleep(Duration::from_millis(500)); // let threads start

        // `None` here means "leave the job as it is" (no cap has been set yet).
        let phases: [(&str, Option<f64>, u64); 4] = [
            ("no cap", None, 3),
            ("cap 25% of machine", Some(25.0), 4),
            ("cap 0.01% (pause)", Some(0.01), 3),
            ("cap lifted", Some(100.0), 3),
        ];
        for (label, cap, seconds) in phases {
            if let Some(percent) = cap {
                // Lifting the cap: first try switching rate control off; if
                // Windows refuses, a 100% cap is the same thing in effect.
                if percent >= 100.0 {
                    match job.set_cpu_cap(None) {
                        Ok(()) => println!("(rate control switched off)"),
                        Err(e) => {
                            println!(
                                "(switching rate control off failed: {}; using a 100% cap)",
                                e.message()
                            );
                            job.set_cpu_cap(Some(100.0)).expect("set 100% cap");
                        }
                    }
                } else {
                    job.set_cpu_cap(Some(percent)).expect("set cpu cap");
                }
            }
            // Skip the first half second: the cap takes a moment to bite.
            std::thread::sleep(Duration::from_millis(500));
            let (cpu0, _) = job.accounting();
            let wall0 = Instant::now();
            std::thread::sleep(Duration::from_secs(seconds));
            let (cpu1, _) = job.accounting();
            let share =
                (cpu1 - cpu0).as_secs_f64() / (wall0.elapsed().as_secs_f64() * cpus as f64) * 100.0;
            println!("{label:<22} measured {share:>6.2}% of machine");
        }
        let _ = child.kill();
        let _ = child.wait();
        job.close();
        println!();
    }

    fn memory_cap_test() {
        println!("== 2. Memory cap of 256 MiB; task tries to take 1024 MiB ==");
        let job = Job::new(Some(256 << 20));
        let mut child = start_worker(&["mem", "32", "1024"]);
        job.assign(&child);
        go(&mut child);

        let stdout = child.stdout.take().expect("worker stdout");
        let mut last = String::new();
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            last = line;
            if last.starts_with("failed") || last.starts_with("done") {
                break;
            }
        }
        let exited = child.try_wait().ok().flatten();
        println!("Task's last report:   {last} (MiB)");
        println!(
            "Task state:           {}",
            match exited {
                Some(status) => format!("exited ({status})"),
                None => "still running; its allocation was refused".to_string(),
            }
        );
        println!(
            "Peak job memory:      {:.0} MiB",
            job.peak_memory() as f64 / (1 << 20) as f64
        );
        let _ = child.kill();
        let _ = child.wait();
        job.close();
        println!();
    }

    fn kill_on_close_test() {
        println!("== 3. Closing the job kills the task and its children ==");
        let job = Job::new(None);
        let mut child = start_worker(&["spawn"]);
        job.assign(&child);
        go(&mut child);

        let stdout = child.stdout.take().expect("worker stdout");
        let mut grandchild_pid = 0u32;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(pid) = line.strip_prefix("grandchild ") {
                grandchild_pid = pid.trim().parse().unwrap_or(0);
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
        let (_, active) = job.accounting();
        println!(
            "Processes in job:     {active} (task pid {}, child-of-task pid {grandchild_pid})",
            child.id()
        );

        let task = is_alive(child.id());
        let grandchild = is_alive(grandchild_pid);
        let start = Instant::now();
        job.close();

        let mut killed_in = None;
        while start.elapsed() < Duration::from_secs(3) {
            let any_alive = [task, grandchild].into_iter().flatten().any(still_running);
            if !any_alive {
                killed_in = Some(start.elapsed());
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        match killed_in {
            Some(t) => println!("Both gone after:      {:.0} ms", t.as_secs_f64() * 1000.0),
            None => println!("FAILED: a process survived 3 s after the job closed"),
        }
        let _ = child.wait();
    }
}
