//! rosfix-pack: builds a knowledge pack from source checkouts.
//!
//! Usage: rosfix-pack --distro jazzy --out crates/ros2-safety-lint/packs/jazzy.json.gz <checkout>...
//! Each checkout is a git clone of a ROS repository at the distribution's branch.

use std::path::PathBuf;
use std::process::Command;

fn git(dir: &PathBuf, args: &[&str]) -> String {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut distro = None;
    let mut out = None;
    let mut dirs = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--distro" => {
                i += 1;
                distro = args.get(i).cloned();
            }
            "--out" => {
                i += 1;
                out = args.get(i).map(PathBuf::from);
            }
            d => dirs.push(PathBuf::from(d)),
        }
        i += 1;
    }
    let (Some(distro), Some(out)) = (distro, out) else {
        eprintln!("usage: rosfix-pack --distro <name> --out <file.json.gz> <checkout>...");
        std::process::exit(2);
    };
    let roots: Vec<(PathBuf, String, String)> = dirs
        .iter()
        .map(|d| {
            let url = git(d, &["config", "--get", "remote.origin.url"]);
            let rev = git(d, &["rev-parse", "HEAD"]);
            (d.clone(), url, rev)
        })
        .collect();
    let pack = rosfix::pack::build(&distro, &roots);
    let (mut exes, mut comps, mut params) = (0, 0, 0);
    for p in pack.packages.values() {
        exes += p.executables.len();
        comps += p.components.len();
        params += p
            .executables
            .values()
            .chain(p.components.values())
            .map(|n| n.params.len())
            .sum::<usize>();
    }
    let bytes = rosfix::pack::encode(&pack);
    std::fs::write(&out, &bytes).expect("write pack");
    eprintln!(
        "{distro}: {} packages, {exes} executables, {comps} components, {params} parameter declarations → {} ({} KiB)",
        pack.packages.len(),
        out.display(),
        bytes.len() / 1024
    );
}
