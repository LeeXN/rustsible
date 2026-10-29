use anyhow::{bail, Context, Result};
use rustsible_protocol::{registry::Builtin, ModuleResult};
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{CStr, CString},
    fs,
    net::{Ipv4Addr, Ipv6Addr},
};

const MIN: &[&str] = &[
    "architecture",
    "system",
    "kernel",
    "kernel_version",
    "machine",
    "hostname",
    "nodename",
    "fqdn",
    "domain",
    "distribution",
    "distribution_version",
    "distribution_major_version",
    "distribution_release",
    "os_family",
    "date_time",
];
const NETWORK: &[&str] = &[
    "interfaces",
    "all_ipv4_addresses",
    "all_ipv6_addresses",
    "default_ipv4",
];
const PLATFORM: &[&str] = &[
    "architecture",
    "system",
    "kernel",
    "kernel_version",
    "machine",
    "hostname",
    "nodename",
    "fqdn",
    "domain",
];
const DISTRIBUTION: &[&str] = &[
    "distribution",
    "distribution_version",
    "distribution_major_version",
    "distribution_release",
    "os_family",
];

fn strings(args: &serde_yaml::Value, key: &str, default: &[&str]) -> Result<Vec<String>> {
    match args.get(key) {
        None => Ok(default.iter().map(|value| (*value).into()).collect()),
        Some(serde_yaml::Value::String(value)) => Ok(value.split(',').map(str::to_owned).collect()),
        Some(serde_yaml::Value::Sequence(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .context("Fact selectors must be strings")
            })
            .collect(),
        _ => bail!("Fact selectors must be a string or list"),
    }
}
fn fields(subset: &str) -> Result<Vec<&'static str>> {
    Ok(match subset {
        "all" => MIN.iter().chain(NETWORK).copied().collect(),
        "min" => MIN.to_vec(),
        "platform" => PLATFORM.to_vec(),
        "distribution" => DISTRIBUTION.to_vec(),
        "network" => NETWORK.to_vec(),
        value => vec![MIN
            .iter()
            .chain(NETWORK)
            .copied()
            .find(|key| {
                *key == value
                    && !matches!(
                        value,
                        "hostname" | "nodename" | "fqdn" | "domain" | "kernel_version"
                    )
            })
            .context("Unsupported native facts subset")?],
    })
}
fn select(subsets: &[String]) -> Result<BTreeSet<&'static str>> {
    for subset in subsets {
        fields(subset.strip_prefix('!').unwrap_or(subset))?;
    }
    let exclude_all = subsets.iter().any(|subset| subset == "!all");
    let exclude_min = subsets.iter().any(|subset| subset == "!min");
    let positive = subsets
        .iter()
        .filter(|subset| !subset.starts_with('!'))
        .collect::<Vec<_>>();
    let mut selected = if !exclude_all
        && (positive.is_empty() || positive.iter().any(|subset| subset.as_str() == "all"))
    {
        fields("all")?
    } else {
        MIN.to_vec()
    }
    .into_iter()
    .collect::<BTreeSet<_>>();
    if exclude_min {
        for key in MIN {
            selected.remove(key);
        }
    }
    for subset in positive {
        if subset != "all" {
            selected.extend(fields(subset)?);
        }
    }
    for subset in subsets
        .iter()
        .filter_map(|subset| subset.strip_prefix('!'))
        .filter(|subset| !matches!(*subset, "all" | "min"))
    {
        for key in fields(subset)? {
            selected.remove(key);
        }
    }
    Ok(selected)
}
pub fn execute(args: &serde_yaml::Value) -> Result<ModuleResult> {
    Builtin::Setup.validate_parameters(args)?;
    let selected = select(&strings(args, "gather_subset", &["all"])?)?;
    let filters = strings(args, "filter", &[])?;
    let filters = filters
        .iter()
        .map(|pattern| CString::new(pattern.as_str()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut facts = Map::new();
    if selected.iter().any(|key| PLATFORM.contains(key)) {
        let mut system: libc::utsname = unsafe { std::mem::zeroed() };
        if unsafe { libc::uname(&mut system) } != 0 {
            return Err(std::io::Error::last_os_error()).context("Cannot read target platform");
        }
        let text = |value: &[libc::c_char]| {
            unsafe { CStr::from_ptr(value.as_ptr()) }
                .to_string_lossy()
                .into_owned()
        };
        let hostname = text(&system.nodename);
        let fqdn = if selected.contains("fqdn") || selected.contains("domain") {
            fqdn(&hostname)
        } else {
            hostname.clone()
        };
        facts.extend(Map::from_iter([
            ("architecture".into(), json!(std::env::consts::ARCH)),
            ("system".into(), json!(text(&system.sysname))),
            ("kernel".into(), json!(text(&system.release))),
            ("kernel_version".into(), json!(text(&system.version))),
            ("machine".into(), json!(text(&system.machine))),
            (
                "hostname".into(),
                json!(hostname.split('.').next().unwrap_or(&hostname)),
            ),
            ("nodename".into(), json!(hostname)),
            (
                "domain".into(),
                json!(fqdn.split_once('.').map_or("", |(_, domain)| domain)),
            ),
            ("fqdn".into(), json!(fqdn)),
        ]));
    }
    if selected.iter().any(|key| DISTRIBUTION.contains(key)) {
        let mut release = distribution(&fs::read_to_string("/etc/os-release").unwrap_or_default());
        // Ansible requests the fuller Debian/CentOS version, which their
        // os-release files intentionally truncate to the major component.
        let hint = match release.get("distribution").and_then(Value::as_str) {
            Some("Debian") => Some("/etc/debian_version"),
            Some("CentOS") => Some("/etc/centos-release"),
            _ => None,
        };
        if let Some(hint) = hint {
            if let Ok(content) = fs::read_to_string(hint) {
                refine_version(&mut release, &content);
            }
        }
        facts.extend(release);
    }
    if selected.contains("date_time") {
        let now = chrono::Local::now();
        facts.insert("date_time".into(), json!({
            "date": now.format("%Y-%m-%d").to_string(), "time": now.format("%H:%M:%S").to_string(),
            "year": now.format("%Y").to_string(), "month": now.format("%m").to_string(), "day": now.format("%d").to_string(),
            "hour": now.format("%H").to_string(), "minute": now.format("%M").to_string(), "second": now.format("%S").to_string(),
            "weekday": now.format("%A").to_string(), "weekday_number": now.format("%w").to_string(),
            "epoch": now.timestamp().to_string(), "iso8601": now.with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        }));
    }
    if selected.iter().any(|key| NETWORK.contains(key)) {
        facts.extend(network()?);
    }
    let mut output = serde_yaml::Mapping::new();
    for (name, value) in facts {
        if !selected.contains(name.as_str()) {
            continue;
        }
        let name = format!("ansible_{name}");
        let candidate = CString::new(name.as_str())?;
        if !filters.is_empty() && !filters.iter().any(|filter| unsafe { libc::fnmatch(filter.as_ptr(), candidate.as_ptr(), libc::FNM_NOESCAPE) } == 0) { continue; }
        output.insert(
            serde_yaml::Value::String(name),
            serde_yaml::to_value(value)?,
        );
    }
    Ok(ModuleResult {
        values: std::collections::HashMap::from([(
            "ansible_facts".into(),
            serde_yaml::Value::Mapping(output),
        )]),
        msg: "Facts gathered".into(),
        ..Default::default()
    })
}

fn fqdn(hostname: &str) -> String {
    // Static workers must preserve aliases from the target's hosts file even
    // when their own libc exposes only the short primary through reverse NSS.
    let hosts = fs::read_to_string("/etc/hosts").unwrap_or_default();
    let records = hosts
        .lines()
        .map(|line| {
            line.split('#')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .collect::<Vec<_>>()
        })
        .filter(|fields| fields.len() > 1)
        .collect::<Vec<_>>();
    if let Some(record) = records.iter().find(|fields| {
        fields[1..]
            .iter()
            .any(|name| name.eq_ignore_ascii_case(hostname))
    }) {
        let names = records
            .iter()
            .filter(|fields| fields[0] == record[0])
            .flat_map(|fields| fields[1..].iter())
            .copied()
            .collect::<Vec<_>>();
        return names
            .iter()
            .find(|name| name.contains('.'))
            .or_else(|| names.first())
            .copied()
            .unwrap_or(hostname)
            .into();
    }
    let Ok(name) = CString::new(hostname) else {
        return hostname.into();
    };
    let hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    let mut addresses = std::ptr::null_mut();
    if unsafe { libc::getaddrinfo(name.as_ptr(), std::ptr::null(), &hints, &mut addresses) } != 0
        || addresses.is_null()
    {
        return hostname.into();
    }
    let mut buffer = [0 as libc::c_char; 1025];
    // The request supervisor bounds DNS time and owns cancellation. Copy the
    // reverse name before releasing libc's address list.
    let status = unsafe {
        libc::getnameinfo(
            (*addresses).ai_addr,
            (*addresses).ai_addrlen,
            buffer.as_mut_ptr(),
            buffer.len() as libc::socklen_t,
            std::ptr::null_mut(),
            0,
            libc::NI_NAMEREQD,
        )
    };
    let result = if status == 0 {
        unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    } else {
        hostname.into()
    };
    unsafe { libc::freeaddrinfo(addresses) };
    result
}

fn distribution(release: &str) -> Map<String, Value> {
    let values = release
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| {
            (
                name,
                rustsible_protocol::tokenize_args(value)
                    .ok()
                    .filter(|values| values.len() == 1)
                    .map(|values| values[0].clone())
                    .unwrap_or_else(|| value.trim_matches('"').into()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let get = |key| values.get(key).map(String::as_str).unwrap_or("");
    let id = get("ID");
    let name = match id {
        "ubuntu" => "Ubuntu",
        "debian" => "Debian",
        "alpine" => "Alpine",
        "rocky" => "Rocky",
        "centos" => "CentOS",
        "rhel" => "RedHat",
        "fedora" => "Fedora",
        "almalinux" => "AlmaLinux",
        "arch" => "Archlinux",
        "opensuse-leap" => "openSUSE Leap",
        "opensuse-tumbleweed" => "openSUSE Tumbleweed",
        "sles" => "SLES",
        _ => {
            if get("NAME").is_empty() {
                "OtherLinux"
            } else {
                get("NAME")
            }
        }
    };
    let ids = format!("{id} {}", get("ID_LIKE"));
    let family = if ids
        .split_whitespace()
        .any(|id| matches!(id, "debian" | "ubuntu"))
    {
        "Debian"
    } else if ids
        .split_whitespace()
        .any(|id| matches!(id, "rhel" | "fedora" | "centos" | "rocky" | "almalinux"))
    {
        "RedHat"
    } else if ids.contains("suse") || ids.contains("sles") {
        "Suse"
    } else if id == "alpine" {
        "Alpine"
    } else if id == "arch" {
        "Archlinux"
    } else {
        name
    };
    let version = if get("VERSION_ID").is_empty() {
        "NA"
    } else {
        get("VERSION_ID")
    };
    let release = if get("NAME").contains("CentOS Stream") {
        "Stream"
    } else if !get("VERSION_CODENAME").is_empty() {
        get("VERSION_CODENAME")
    } else if !get("UBUNTU_CODENAME").is_empty() {
        get("UBUNTU_CODENAME")
    } else {
        get("VERSION")
            .rsplit_once('(')
            .and_then(|(_, tail)| tail.strip_suffix(')'))
            .filter(|value| !value.is_empty())
            .unwrap_or("NA")
    };
    Map::from_iter([
        (
            "distribution".into(),
            json!(if name.is_empty() { "NA" } else { name }),
        ),
        ("distribution_version".into(), json!(version)),
        (
            "distribution_major_version".into(),
            json!(version.split('.').next().unwrap_or("NA")),
        ),
        ("distribution_release".into(), json!(release)),
        ("os_family".into(), json!(family)),
    ])
}

fn refine_version(facts: &mut Map<String, Value>, hint: &str) {
    let distribution = facts
        .get("distribution")
        .and_then(Value::as_str)
        .unwrap_or("");
    let candidate = if distribution == "Debian" {
        hint.trim().to_owned()
    } else {
        hint.split_whitespace()
            .find(|word| {
                word.chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_digit())
            })
            .unwrap_or("")
            .split('.')
            .take(2)
            .collect::<Vec<_>>()
            .join(".")
    };
    if !candidate.is_empty()
        && candidate
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
    {
        facts.insert(
            "distribution_major_version".into(),
            json!(candidate.split('.').next().unwrap_or("NA")),
        );
        facts.insert("distribution_version".into(), json!(candidate));
    }
}

fn network() -> Result<Map<String, Value>> {
    let mut addresses = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut addresses) } != 0 {
        return Err(std::io::Error::last_os_error()).context("Cannot read target interfaces");
    }
    let mut interfaces = BTreeSet::new();
    let mut ipv4 = BTreeSet::new();
    let mut ipv6 = BTreeSet::new();
    let mut by_interface = BTreeMap::new();
    let mut current = addresses;
    while !current.is_null() {
        let entry = unsafe { &*current };
        let name = unsafe { CStr::from_ptr(entry.ifa_name) }
            .to_string_lossy()
            .into_owned();
        interfaces.insert(name.clone());
        if !entry.ifa_addr.is_null() {
            match unsafe { (*entry.ifa_addr).sa_family } as i32 {
                libc::AF_INET => {
                    let address = Ipv4Addr::from(
                        unsafe {
                            (*(entry.ifa_addr as *const libc::sockaddr_in))
                                .sin_addr
                                .s_addr
                        }
                        .to_ne_bytes(),
                    );
                    if !address.is_loopback() {
                        ipv4.insert(address.to_string());
                        let mut info = Map::from_iter([
                            ("address".into(), json!(address.to_string())),
                            ("interface".into(), json!(name)),
                        ]);
                        if !entry.ifa_netmask.is_null() {
                            let mask = Ipv4Addr::from(
                                unsafe {
                                    (*(entry.ifa_netmask as *const libc::sockaddr_in))
                                        .sin_addr
                                        .s_addr
                                }
                                .to_ne_bytes(),
                            );
                            let network = u32::from(address) & u32::from(mask);
                            info.insert("netmask".into(), json!(mask.to_string()));
                            info.insert(
                                "network".into(),
                                json!(Ipv4Addr::from(network).to_string()),
                            );
                            info.insert(
                                "broadcast".into(),
                                json!(Ipv4Addr::from(network | !u32::from(mask)).to_string()),
                            );
                        }
                        by_interface.entry(name.clone()).or_insert(info);
                    }
                }
                libc::AF_INET6 => {
                    let address = Ipv6Addr::from(unsafe {
                        (*(entry.ifa_addr as *const libc::sockaddr_in6))
                            .sin6_addr
                            .s6_addr
                    });
                    if !address.is_loopback() {
                        ipv6.insert(address.to_string());
                    }
                }
                _ => {}
            }
        }
        current = entry.ifa_next;
    }
    unsafe { libc::freeifaddrs(addresses) };
    let route = fs::read_to_string("/proc/net/route")
        .unwrap_or_default()
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 8
                || fields[1] != "00000000"
                || u32::from_str_radix(fields[3], 16).ok()? & 1 == 0
            {
                return None;
            }
            Some((
                fields[6].parse::<u32>().ok()?,
                fields[0].to_owned(),
                u32::from_str_radix(fields[2], 16).ok()?,
            ))
        })
        .min_by_key(|(metric, _, _)| *metric);
    let mut default = Map::new();
    if let Some((_, interface, gateway)) = route {
        if let Some(info) = by_interface.remove(&interface) {
            default = info;
            default.insert(
                "gateway".into(),
                json!(Ipv4Addr::from(gateway.to_le_bytes()).to_string()),
            );
            if let Ok(mac) = fs::read_to_string(format!("/sys/class/net/{interface}/address")) {
                default.insert("macaddress".into(), json!(mac.trim()));
            }
            if let Ok(mtu) = fs::read_to_string(format!("/sys/class/net/{interface}/mtu")) {
                if let Ok(mtu) = mtu.trim().parse::<u64>() {
                    default.insert("mtu".into(), json!(mtu));
                }
            }
        }
    }
    Ok(Map::from_iter([
        ("interfaces".into(), json!(interfaces)),
        ("all_ipv4_addresses".into(), json!(ipv4)),
        ("all_ipv6_addresses".into(), json!(ipv6)),
        ("default_ipv4".into(), Value::Object(default)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subset_selection_and_distribution_mapping_are_explicit() {
        let select_names = |names: &[&str]| {
            select(&names.iter().map(|name| (*name).into()).collect::<Vec<_>>()).unwrap()
        };
        assert_eq!(
            select_names(&["!all", "!min", "date_time"]),
            BTreeSet::from(["date_time"])
        );
        assert!(select_names(&["!all", "!min"]).is_empty());
        assert!(!select_names(&["!network"]).contains("interfaces"));
        assert!(select_names(&["all"]).contains("default_ipv4"));
        assert!(select(&["python".into()]).is_err());
        let facts = distribution("NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\nVERSION_ID=\"24.04\"\nVERSION_CODENAME=noble");
        assert_eq!(facts["distribution"], "Ubuntu");
        assert_eq!(facts["os_family"], "Debian");
        assert_eq!(facts["distribution_major_version"], "24");
        assert_eq!(
            distribution("ID=rocky\nID_LIKE=\"rhel centos fedora\"")["os_family"],
            "RedHat"
        );
        let mut debian = distribution("ID=debian\nVERSION_ID=12");
        refine_version(&mut debian, "12.13\n");
        assert_eq!(debian["distribution_version"], "12.13");
        let mut centos = distribution("ID=centos\nVERSION_ID=7\nVERSION=\"7 (Core)\"");
        refine_version(&mut centos, "CentOS Linux release 7.9.2009 (Core)");
        assert_eq!(centos["distribution_version"], "7.9");
        assert_eq!(centos["distribution_release"], "Core");
        assert_eq!(distribution("ID=arch")["distribution_version"], "NA");
    }
}
