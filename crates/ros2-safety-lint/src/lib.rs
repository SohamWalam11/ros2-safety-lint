//! rosfix — static checker for silent ROS 2 misconfigurations.
//!
//! Pipeline: [`workspace::index`] extracts facts from every relevant file,
//! [`resolve::build`] links launch files, parameter files and source code into a
//! launch graph, and the [`agents`] (one per problem domain) read facts + graph to
//! produce [`diag::Diagnostic`]s. [`fix`] verifies and applies safe fixes.

pub mod agents;
pub mod assist;
pub mod catalog;
pub mod config;
pub mod diag;
pub mod docs;
pub mod doctor;
pub mod engine;
pub mod extract;
pub mod facts;
pub mod fix;
pub mod html;
pub mod init;
pub mod installed;
pub mod live;
pub mod migrate;
pub mod nodl;
pub mod pack;
pub mod policy;
pub mod report;
pub mod resolve;
pub mod rules;
pub mod sandbox;
pub mod session;
pub mod source;
pub mod status;
pub mod triage;
pub mod util;
pub mod workspace;

#[cfg(test)]
pub(crate) mod testutil {
    use crate::config::Config;
    use crate::diag::Diagnostic;
    use crate::engine::{analyze, Options};

    /// Writes files into a temporary workspace and runs the full analysis.
    pub fn analyze_files(files: &[(&str, &str)]) -> Vec<Diagnostic> {
        let dir = tempfile::tempdir().expect("tempdir");
        for (rel, text) in files {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, text).unwrap();
        }
        let a = analyze(dir.path(), &Config::default(), &Options::default());
        a.diagnostics
    }

    /// Like `analyze_files`, with a fake installed ROS prefix; also returns the analysis.
    pub fn analyze_with_ros(files: &[(&str, &str)]) -> crate::engine::Analysis {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws = dir.path().join("ws");
        for (rel, text) in files {
            let p = ws.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, text).unwrap();
        }
        let prefix = crate::installed::tests::fake_prefix(dir.path());
        let opts = Options {
            prefixes: vec![prefix],
            ..Default::default()
        };
        let a = analyze(&ws, &Config::default(), &opts);
        std::mem::forget(dir); // keep files alive for the returned analysis' paths
        a
    }
}

#[cfg(test)]
mod installed_tests {
    use crate::testutil::analyze_with_ros;

    #[test]
    fn installed_packages_are_understood() {
        let a = analyze_with_ros(&[
            ("p/package.xml", "<package format=\"3\"><name>p</name><exec_depend>nav2_controller</exec_depend><exec_depend>nav2_bringup</exec_depend><exec_depend>not_a_pkg</exec_depend></package>"),
            (
                "p/launch/a.launch.xml",
                "<launch>\n<node pkg=\"nav2_controller\" exec=\"controller_srever\"/>\n<node pkg=\"not_a_pkg\" exec=\"x\"/>\n<include file=\"$(find-pkg-share nav2_bringup)/launch/bringup.launch.py\"/>\n</launch>",
            ),
        ]);
        let d = &a.diagnostics;
        assert!(
            d.iter()
                .any(|x| x.rule == "L001" && x.message.contains("controller_srever")),
            "{d:#?}"
        );
        assert!(
            d.iter()
                .any(|x| x.rule == "L001" && x.message.contains("not_a_pkg")),
            "{d:#?}"
        );
        // The included installed launch file was loaded and its node is in the graph.
        assert!(
            a.graph
                .instances()
                .any(|(_, _, n)| n.fq_name.as_deref() == Some("/controller_server")),
            "{:#?}",
            a.graph.roots
        );
        // Nothing is reported inside installed files.
        assert!(d
            .iter()
            .all(|x| !a.index.external_files.contains(&x.loc.file)));
    }
}
