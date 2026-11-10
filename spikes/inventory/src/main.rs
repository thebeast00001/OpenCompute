//! Spike 4: can we read accurate hardware inventory, live metrics and
//! user-presence signals on Windows, from Rust, without admin rights?
//!
//! Questions:
//! 1. Does VRAM read correctly? (WMI's `AdapterRAM` reports 4 GB for an 8 GB card.)
//! 2. Do the presence APIs the sharing policy needs (idle time, power, full
//!    screen) work from an ordinary process?
//! 3. What does one metrics sample cost? The agent samples once per second.

use std::time::Instant;

use sysinfo::{Components, Disks, Networks, System};

fn gib(bytes: u64) -> f64 {
    bytes as f64 / (1u64 << 30) as f64
}

fn main() {
    system_section();
    let mut sys = cpu_and_memory_section();
    storage_and_network_section();
    let nvml = nvidia_section();
    #[cfg(windows)]
    win::sections();
    sampling_cost_section(&mut sys, nvml.as_ref());
}

fn system_section() {
    println!("== System ==");
    println!(
        "OS:        {} {}",
        System::name().unwrap_or_default(),
        System::os_version().unwrap_or_default()
    );
    println!(
        "Kernel:    {}",
        System::kernel_version().unwrap_or_default()
    );
    println!("Arch:      {}", System::cpu_arch());
}

fn cpu_and_memory_section() -> System {
    let mut sys = System::new_all().expect("sysinfo could not read this system");
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_all();

    println!("\n== CPU ==");
    let cpus = sys.cpus();
    println!(
        "Model:     {}",
        cpus.first().map(|c| c.brand().trim()).unwrap_or("unknown")
    );
    println!(
        "Cores:     {} physical, {} logical",
        System::physical_core_count().map_or("?".to_string(), |n| n.to_string()),
        cpus.len()
    );
    println!("Usage:     {:.1}%", sys.global_cpu_usage());

    println!("\n== Memory ==");
    println!("Total:     {:.1} GiB", gib(sys.total_memory()));
    println!("Available: {:.1} GiB", gib(sys.available_memory()));
    sys
}

fn storage_and_network_section() {
    println!("\n== Disks ==");
    match Disks::new_with_refreshed_list() {
        Ok(disks) => {
            for disk in &disks {
                println!(
                    "{:<10} {:<5} {:>7.1} GiB total, {:>7.1} GiB free",
                    disk.mount_point().display(),
                    disk.kind().to_string(),
                    gib(disk.total_space()),
                    gib(disk.available_space())
                );
            }
        }
        Err(e) => println!("unavailable: {e}"),
    }

    println!("\n== Network interfaces seen by sysinfo ==");
    match Networks::new_with_refreshed_list() {
        Ok(networks) => println!(
            "{} interfaces (no link speed available from this source)",
            networks.len()
        ),
        Err(e) => println!("unavailable: {e}"),
    }

    println!("\n== Temperature sensors seen by sysinfo ==");
    match Components::new_with_refreshed_list() {
        Ok(components) => {
            println!("{} sensors", components.len());
            for c in &components {
                println!("{:<30} {:?} C", c.label(), c.temperature());
            }
        }
        Err(e) => println!("unavailable: {e}"),
    }

    // sysinfo gained a GPU module recently. Compare it with NVML and DXGI
    // below before trusting it.
    println!("\n== GPUs seen by sysinfo ==");
    match sysinfo::Gpus::new_with_refreshed_list() {
        Ok(gpus) => {
            println!("{} GPUs", gpus.len());
            for g in &gpus {
                println!(
                    "{:<40} vendor {:?}  total {:?} GiB  used {:?} GiB  usage {:?}%",
                    g.model().unwrap_or("unknown"),
                    g.vendor(),
                    g.total_memory().map(gib),
                    g.used_memory().map(gib),
                    g.usage()
                );
            }
        }
        Err(e) => println!("unavailable: {e}"),
    }
}

fn nvidia_section() -> Option<nvml_wrapper::Nvml> {
    use nvml_wrapper::enum_wrappers::device::TemperatureSensor;

    println!("\n== NVIDIA (NVML) ==");
    let nvml = match nvml_wrapper::Nvml::init() {
        Ok(n) => n,
        Err(e) => {
            println!("NVML not available: {e}");
            return None;
        }
    };
    println!(
        "Driver:    {}",
        nvml.sys_driver_version().unwrap_or_default()
    );
    if let Ok(v) = nvml.sys_cuda_driver_version() {
        println!(
            "CUDA:      {}.{}",
            nvml_wrapper::cuda_driver_version_major(v),
            nvml_wrapper::cuda_driver_version_minor(v)
        );
    }
    let count = nvml.device_count().unwrap_or(0);
    for i in 0..count {
        let Ok(dev) = nvml.device_by_index(i) else {
            continue;
        };
        println!("GPU {i}:     {}", dev.name().unwrap_or_default());
        if let Ok(mem) = dev.memory_info() {
            println!(
                "VRAM:      {:.2} GiB total, {:.2} GiB used (system-wide)",
                gib(mem.total),
                gib(mem.used)
            );
        }
        if let Ok(u) = dev.utilization_rates() {
            println!("Load:      {}% GPU, {}% memory controller", u.gpu, u.memory);
        }
        if let Ok(t) = dev.temperature(TemperatureSensor::Gpu) {
            println!("Temp:      {t} C");
        }
        match dev.power_usage() {
            Ok(mw) => println!("Power:     {:.1} W", mw as f64 / 1000.0),
            Err(e) => println!("Power:     not reported ({e})"),
        }
    }
    Some(nvml)
}

/// How expensive is the once-per-second sample the agent will take?
fn sampling_cost_section(sys: &mut System, nvml: Option<&nvml_wrapper::Nvml>) {
    use nvml_wrapper::enum_wrappers::device::TemperatureSensor;

    println!("\n== Cost of one metrics sample ==");
    const ROUNDS: u32 = 50;
    let start = Instant::now();
    for _ in 0..ROUNDS {
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        if let Some(dev) = nvml.and_then(|n| n.device_by_index(0).ok()) {
            let _ = dev.memory_info();
            let _ = dev.utilization_rates();
            let _ = dev.temperature(TemperatureSensor::Gpu);
        }
    }
    let per_sample = start.elapsed() / ROUNDS;
    println!(
        "CPU + memory + GPU: {:.3} ms per sample ({:.3}% of one core at 1 Hz)",
        per_sample.as_secs_f64() * 1000.0,
        per_sample.as_secs_f64() * 100.0
    );
}

#[cfg(windows)]
mod win {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
        DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter3, IDXGIFactory1,
    };
    use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows::Win32::UI::Shell::SHQueryUserNotificationState;
    use windows::core::Interface;

    use super::gib;

    fn wide(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    pub fn sections() {
        gpus_via_dxgi();
        link_speeds();
        presence();
    }

    /// Works for every GPU vendor on Windows, with no vendor library.
    fn gpus_via_dxgi() {
        println!("\n== GPUs (DXGI, all vendors) ==");
        // SAFETY: plain COM calls; every out-value is checked before use.
        unsafe {
            let factory: IDXGIFactory1 = match CreateDXGIFactory1() {
                Ok(f) => f,
                Err(e) => {
                    println!("DXGI not available: {e}");
                    return;
                }
            };
            let mut index = 0;
            while let Ok(adapter) = factory.EnumAdapters1(index) {
                index += 1;
                let Ok(desc) = adapter.GetDesc1() else {
                    continue;
                };
                let software = desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0;
                println!(
                    "{:<40} vendor {:#06x}  dedicated VRAM {:>5.2} GiB  shared {:>5.2} GiB{}",
                    wide(&desc.Description),
                    desc.VendorId,
                    gib(desc.DedicatedVideoMemory as u64),
                    gib(desc.SharedSystemMemory as u64),
                    if software { "  (software)" } else { "" }
                );
                if let Ok(adapter3) = adapter.cast::<IDXGIAdapter3>() {
                    let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
                    if adapter3
                        .QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info)
                        .is_ok()
                    {
                        println!(
                            "{:<40} budget for this process {:.2} GiB",
                            "",
                            gib(info.Budget)
                        );
                    }
                }
            }
        }
    }

    /// Link rate per physical adapter: what the scheduler needs before it has
    /// measured a link itself.
    fn link_speeds() {
        println!("\n== Physical network adapters (IP Helper) ==");
        // SAFETY: the table is allocated by GetIfTable2 and freed below; rows
        // are read only while the table is alive.
        unsafe {
            let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
            if GetIfTable2(&mut table).is_err() || table.is_null() {
                println!("GetIfTable2 failed");
                return;
            }
            let rows =
                std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
            for row in rows {
                // Bit 2 of the flags is ConnectorPresent: a real adapter.
                let physical = row.InterfaceAndOperStatusFlags._bitfield & 0b100 != 0;
                if !physical {
                    continue;
                }
                let up = row.OperStatus == IfOperStatusUp;
                println!(
                    "{:<28} {:<4} tx {:>7.0} Mbps  rx {:>7.0} Mbps  ({})",
                    wide(&row.Alias),
                    if up { "up" } else { "down" },
                    row.TransmitLinkSpeed as f64 / 1e6,
                    row.ReceiveLinkSpeed as f64 / 1e6,
                    wide(&row.Description)
                );
            }
            FreeMibTable(table.cast());
        }
    }

    /// Signals the sharing policy uses to decide when to yield to the owner.
    fn presence() {
        println!("\n== Presence signals ==");
        // SAFETY: each call fills a caller-owned struct of the declared size.
        unsafe {
            let mut last = LASTINPUTINFO {
                cbSize: size_of::<LASTINPUTINFO>() as u32,
                dwTime: 0,
            };
            if GetLastInputInfo(&mut last).as_bool() {
                let idle_ms = GetTickCount().wrapping_sub(last.dwTime);
                println!("Idle for:        {:.1} s", idle_ms as f64 / 1000.0);
            } else {
                println!("Idle for:        unavailable");
            }

            let mut power = SYSTEM_POWER_STATUS::default();
            if GetSystemPowerStatus(&mut power).is_ok() {
                let source = match power.ACLineStatus {
                    0 => "battery",
                    1 => "mains",
                    _ => "unknown",
                };
                let battery = if power.BatteryLifePercent == 255 {
                    "no battery reported".to_string()
                } else {
                    format!("{}%", power.BatteryLifePercent)
                };
                println!("Power source:    {source}, battery {battery}");
            }

            match SHQueryUserNotificationState() {
                Ok(state) => println!(
                    "Notification state: {} (1 = not present, 2 = busy, 3 = full-screen D3D, 4 = presentation, 5 = normal)",
                    state.0
                ),
                Err(e) => println!("Notification state: unavailable ({e})"),
            }
        }
    }
}
