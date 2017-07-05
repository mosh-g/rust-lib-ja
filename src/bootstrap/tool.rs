// Copyright 2017 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use Mode;
use builder::{Step, Builder};
use util::{exe, add_lib_path};
use compile::{self, libtest_stamp, libstd_stamp, librustc_stamp, Rustc};
use native;
use channel::GitInfo;

//// ========================================================================
//// Build tools
////
//// Tools used during the build system but not shipped
//// "pseudo rule" which represents completely cleaning out the tools dir in
//// one stage. This needs to happen whenever a dependency changes (e.g.
//// libstd, libtest, librustc) and all of the tool compilations above will
//// be sequenced after this rule.
//rules.build("maybe-clean-tools", "path/to/nowhere")
//     .after("librustc-tool")
//     .after("libtest-tool")
//     .after("libstd-tool");
//
//rules.build("librustc-tool", "path/to/nowhere")
//     .dep(|s| s.name("librustc"))
//     .run(move |s| compile::maybe_clean_tools(build, s.stage, s.target, Mode::Librustc));
//rules.build("libtest-tool", "path/to/nowhere")
//     .dep(|s| s.name("libtest"))
//     .run(move |s| compile::maybe_clean_tools(build, s.stage, s.target, Mode::Libtest));
//rules.build("libstd-tool", "path/to/nowhere")
//     .dep(|s| s.name("libstd"))
//     .run(move |s| compile::maybe_clean_tools(build, s.stage, s.target, Mode::Libstd));
//

#[derive(Serialize)]
pub struct CleanTools<'a> {
    pub stage: u32,
    pub target: &'a str,
    pub mode: Mode,
}

impl<'a> Step<'a> for CleanTools<'a> {
    type Output = ();

    /// Build a tool in `src/tools`
    ///
    /// This will build the specified tool with the specified `host` compiler in
    /// `stage` into the normal cargo output directory.
    fn run(self, builder: &Builder) {
        let build = builder.build;
        let stage = self.stage;
        let target = self.target;
        let mode = self.mode;

        let compiler = builder.compiler(stage, &build.build);

        let stamp = match mode {
            Mode::Libstd => libstd_stamp(build, compiler, target),
            Mode::Libtest => libtest_stamp(build, compiler, target),
            Mode::Librustc => librustc_stamp(build, compiler, target),
            _ => panic!(),
        };
        let out_dir = build.cargo_out(compiler, Mode::Tool, target);
        build.clear_if_dirty(&out_dir, &stamp);
    }
}

#[derive(Serialize)]
pub struct ToolBuild<'a> {
    pub stage: u32,
    pub target: &'a str,
    pub tool: &'a str,
    pub mode: Mode,
}

impl<'a> Step<'a> for ToolBuild<'a> {
    type Output = PathBuf;

    /// Build a tool in `src/tools`
    ///
    /// This will build the specified tool with the specified `host` compiler in
    /// `stage` into the normal cargo output directory.
    fn run(self, builder: &Builder) -> PathBuf {
        let build = builder.build;
        let stage = self.stage;
        let target = self.target;
        let tool = self.tool;

        let compiler = builder.compiler(stage, &build.build);
        builder.ensure(CleanTools { stage, target, mode: self.mode });
        match self.mode {
            Mode::Libstd => builder.ensure(compile::Std { compiler, target }),
            Mode::Libtest => builder.ensure(compile::Test { compiler, target }),
            Mode::Librustc => builder.ensure(compile::Rustc { compiler, target }),
            Mode::Tool => panic!("unexpected Mode::Tool for tool build")
        }

        let _folder = build.fold_output(|| format!("stage{}-{}", stage, tool));
        println!("Building stage{} tool {} ({})", stage, tool, target);

        let mut cargo = build.cargo(compiler, Mode::Tool, target, "build");
        let dir = build.src.join("src/tools").join(tool);
        cargo.arg("--manifest-path").arg(dir.join("Cargo.toml"));

        // We don't want to build tools dynamically as they'll be running across
        // stages and such and it's just easier if they're not dynamically linked.
        cargo.env("RUSTC_NO_PREFER_DYNAMIC", "1");

        if let Some(dir) = build.openssl_install_dir(target) {
            cargo.env("OPENSSL_STATIC", "1");
            cargo.env("OPENSSL_DIR", dir);
            cargo.env("LIBZ_SYS_STATIC", "1");
        }

        cargo.env("CFG_RELEASE_CHANNEL", &build.config.channel);

        let info = GitInfo::new(&dir);
        if let Some(sha) = info.sha() {
            cargo.env("CFG_COMMIT_HASH", sha);
        }
        if let Some(sha_short) = info.sha_short() {
            cargo.env("CFG_SHORT_COMMIT_HASH", sha_short);
        }
        if let Some(date) = info.commit_date() {
            cargo.env("CFG_COMMIT_DATE", date);
        }

        build.run(&mut cargo);
        build.cargo_out(compiler, Mode::Tool, target).join(exe(tool, compiler.host))
    }
}

macro_rules! tool {
    ($($name:ident, $path:expr, $tool_name:expr, $mode:expr;)+) => {
        #[derive(Copy, Clone)]
        pub enum Tool {
            $(
                $name,
            )+
        }

        impl<'a> Builder<'a> {
            pub fn tool_exe(&self, tool: Tool) -> PathBuf {
                match tool {
                    $(Tool::$name =>
                        self.ensure($name {
                            stage: 0,
                            target: &self.build.build,
                        }),
                    )+
                }
            }
        }

        $(
        #[derive(Serialize)]
        pub struct $name<'a> {
            pub stage: u32,
            pub target: &'a str,
        }

        impl<'a> Step<'a> for $name<'a> {
            type Output = PathBuf;

            fn should_run(_builder: &Builder, path: &Path) -> bool {
                path.ends_with($path)
            }

            fn make_run(builder: &Builder, _path: Option<&Path>, _host: &str, target: &str) {
                builder.ensure($name {
                    stage: builder.top_stage,
                    target,
                });
            }

            fn run(self, builder: &Builder) -> PathBuf {
                builder.ensure(ToolBuild {
                    stage: self.stage,
                    target: self.target,
                    tool: $tool_name,
                    mode: $mode,
                })
            }
        }
        )+
    }
}

tool!(
    // rules.build("tool-rustbook", "src/tools/rustbook")
    //      .dep(|s| s.name("maybe-clean-tools"))
    //      .dep(|s| s.name("librustc-tool"))
    //      .run(move |s| compile::tool(build, s.stage, s.target, "rustbook"));
    Rustbook, "src/tools/rustbook", "rustbook", Mode::Librustc;
    // rules.build("tool-error-index", "src/tools/error_index_generator")
    //      .dep(|s| s.name("maybe-clean-tools"))
    //      .dep(|s| s.name("librustc-tool"))
    //      .run(move |s| compile::tool(build, s.stage, s.target, "error_index_generator"));
    ErrorIndex, "src/tools/error_index_generator", "error_index_generator", Mode::Librustc;
    // rules.build("tool-unstable-book-gen", "src/tools/unstable-book-gen")
    //      .dep(|s| s.name("maybe-clean-tools"))
    //      .dep(|s| s.name("libstd-tool"))
    //      .run(move |s| compile::tool(build, s.stage, s.target, "unstable-book-gen"));
    UnstableBookGen, "src/tools/unstable-book-gen", "unstable-book-gen", Mode::Libstd;
    // rules.build("tool-tidy", "src/tools/tidy")
    //      .dep(|s| s.name("maybe-clean-tools"))
    //      .dep(|s| s.name("libstd-tool"))
    //      .run(move |s| compile::tool(build, s.stage, s.target, "tidy"));
    Tidy, "src/tools/tidy", "tidy", Mode::Libstd;
    // rules.build("tool-linkchecker", "src/tools/linkchecker")
    //      .dep(|s| s.name("maybe-clean-tools"))
    //      .dep(|s| s.name("libstd-tool"))
    //      .run(move |s| compile::tool(build, s.stage, s.target, "linkchecker"));
    Linkchecker, "src/tools/linkchecker", "linkchecker", Mode::Libstd;
    // rules.build("tool-cargotest", "src/tools/cargotest")
    //      .dep(|s| s.name("maybe-clean-tools"))
    //      .dep(|s| s.name("libstd-tool"))
    //      .run(move |s| compile::tool(build, s.stage, s.target, "cargotest"));
    CargoTest, "src/tools/cargotest", "cargotest", Mode::Libstd;
    // rules.build("tool-compiletest", "src/tools/compiletest")
    //      .dep(|s| s.name("maybe-clean-tools"))
    //      .dep(|s| s.name("libtest-tool"))
    //      .run(move |s| compile::tool(build, s.stage, s.target, "compiletest"));
    Compiletest, "src/tools/compiletest", "compiletest", Mode::Libtest;
    // rules.build("tool-build-manifest", "src/tools/build-manifest")
    //      .dep(|s| s.name("maybe-clean-tools"))
    //      .dep(|s| s.name("libstd-tool"))
    //      .run(move |s| compile::tool(build, s.stage, s.target, "build-manifest"));
    BuildManifest, "src/tools/build-manifest", "build-manifest", Mode::Libstd;
    // rules.build("tool-remote-test-server", "src/tools/remote-test-server")
    //      .dep(|s| s.name("maybe-clean-tools"))
    //      .dep(|s| s.name("libstd-tool"))
    //      .run(move |s| compile::tool(build, s.stage, s.target, "remote-test-server"));
    RemoteTestServer, "src/tools/remote-test-server", "remote-test-server", Mode::Libstd;
    // rules.build("tool-remote-test-client", "src/tools/remote-test-client")
    //      .dep(|s| s.name("maybe-clean-tools"))
    //      .dep(|s| s.name("libstd-tool"))
    //      .run(move |s| compile::tool(build, s.stage, s.target, "remote-test-client"));
    RemoteTestClient, "src/tools/remote-test-client", "remote-test-client", Mode::Libstd;
    // rules.build("tool-rust-installer", "src/tools/rust-installer")
    //      .dep(|s| s.name("maybe-clean-tools"))
    //      .dep(|s| s.name("libstd-tool"))
    //      .run(move |s| compile::tool(build, s.stage, s.target, "rust-installer"));
    RustInstaller, "src/tools/rust-installer", "rust-installer", Mode::Libstd;
);

// rules.build("tool-cargo", "src/tools/cargo")
//      .host(true)
//      .default(build.config.extended)
//      .dep(|s| s.name("maybe-clean-tools"))
//      .dep(|s| s.name("libstd-tool"))
//      .dep(|s| s.stage(0).host(s.target).name("openssl"))
//      .dep(move |s| {
//          // Cargo depends on procedural macros, which requires a full host
//          // compiler to be available, so we need to depend on that.
//          s.name("librustc-link")
//           .target(&build.build)
//           .host(&build.build)
//      })
//      .run(move |s| compile::tool(build, s.stage, s.target, "cargo"));
#[derive(Serialize)]
pub struct Cargo<'a> {
    pub stage: u32,
    pub target: &'a str,
}

impl<'a> Step<'a> for Cargo<'a> {
    type Output = PathBuf;
    const DEFAULT: bool = true;
    const ONLY_HOSTS: bool = true;

    fn should_run(_builder: &Builder, path: &Path) -> bool {
        path.ends_with("src/tools/cargo")
    }

    fn make_run(builder: &Builder, path: Option<&Path>, _host: &str, target: &str) {
        if path.is_none() && !builder.build.config.extended {
            return;
        }
        builder.ensure(Cargo {
            stage: builder.top_stage,
            target,
        });
    }

    fn run(self, builder: &Builder) -> PathBuf {
        builder.ensure(native::Openssl {
            target: self.target,
        });
        // Cargo depends on procedural macros, which requires a full host
        // compiler to be available, so we need to depend on that.
        builder.ensure(Rustc {
            compiler: builder.compiler(builder.top_stage, &builder.build.build),
            target: &builder.build.build,
        });
        builder.ensure(ToolBuild {
            stage: self.stage,
            target: self.target,
            tool: "cargo",
            mode: Mode::Libstd,
        })
    }
}

// rules.build("tool-rls", "src/tools/rls")
//      .host(true)
//      .default(build.config.extended)
//      .dep(|s| s.name("librustc-tool"))
//      .dep(|s| s.stage(0).host(s.target).name("openssl"))
//      .dep(move |s| {
//          // rls, like cargo, uses procedural macros
//          s.name("librustc-link")
//           .target(&build.build)
//           .host(&build.build)
//      })
//      .run(move |s| compile::tool(build, s.stage, s.target, "rls"));
//
#[derive(Serialize)]
pub struct Rls<'a> {
    pub stage: u32,
    pub target: &'a str,
}

impl<'a> Step<'a> for Rls<'a> {
    type Output = PathBuf;
    const DEFAULT: bool = true;
    const ONLY_HOSTS: bool = true;

    fn should_run(_builder: &Builder, path: &Path) -> bool {
        path.ends_with("src/tools/rls")
    }

    fn make_run(builder: &Builder, path: Option<&Path>, _host: &str, target: &str) {
        if path.is_none() && !builder.build.config.extended {
            return;
        }
        builder.ensure(Cargo {
            stage: builder.top_stage,
            target,
        });
    }

    fn run(self, builder: &Builder) -> PathBuf {
        builder.ensure(native::Openssl {
            target: self.target,
        });
        // RLS depends on procedural macros, which requires a full host
        // compiler to be available, so we need to depend on that.
        builder.ensure(Rustc {
            compiler: builder.compiler(builder.top_stage, &builder.build.build),
            target: &builder.build.build,
        });
        builder.ensure(ToolBuild {
            stage: self.stage,
            target: self.target,
            tool: "rls",
            mode: Mode::Librustc,
        })
    }
}
