// Copyright 2016 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Implementation of the test-related targets of the build system.
//!
//! This file implements the various regression test suites that we execute on
//! our CI.

use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::iter;
use std::fmt;
use std::fs::{self, File};
use std::path::{PathBuf, Path};
use std::process::Command;
use std::io::Read;

use build_helper::{self, output};

use {Build, Mode};
use dist;
use util::{self, dylib_path, dylib_path_var};

use compile;
use native;
use builder::{Kind, ShouldRun, Builder, Compiler, Step};
use tool::{self, Tool};
use cache::{INTERNER, Interned};

const ADB_TEST_DIR: &str = "/data/tmp/work";

/// The two modes of the test runner; tests or benchmarks.
#[derive(Debug, PartialEq, Eq, Hash, Copy, Clone)]
pub enum TestKind {
    /// Run `cargo test`
    Test,
    /// Run `cargo bench`
    Bench,
}

impl TestKind {
    // Return the cargo subcommand for this test kind
    fn subcommand(self) -> &'static str {
        match self {
            TestKind::Test => "test",
            TestKind::Bench => "bench",
        }
    }
}

impl fmt::Display for TestKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match *self {
            TestKind::Test => "Testing",
            TestKind::Bench => "Benchmarking",
        })
    }
}

fn try_run(build: &Build, cmd: &mut Command) {
    if !build.fail_fast {
        if !build.try_run(cmd) {
            let failures = build.delayed_failures.get();
            build.delayed_failures.set(failures + 1);
        }
    } else {
        build.run(cmd);
    }
}

fn try_run_quiet(build: &Build, cmd: &mut Command) {
    if !build.fail_fast {
        if !build.try_run_quiet(cmd) {
            let failures = build.delayed_failures.get();
            build.delayed_failures.set(failures + 1);
        }
    } else {
        build.run_quiet(cmd);
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Linkcheck {
    host: Interned<String>,
}

impl Step for Linkcheck {
    type Output = ();
    const ONLY_HOSTS: bool = true;
    const DEFAULT: bool = true;

    /// Runs the `linkchecker` tool as compiled in `stage` by the `host` compiler.
    ///
    /// This tool in `src/tools` will verify the validity of all our links in the
    /// documentation to ensure we don't have a bunch of dead ones.
    fn run(self, builder: &Builder) {
        let build = builder.build;
        let host = self.host;

        println!("Linkcheck ({})", host);

        builder.default_doc(None);

        let _time = util::timeit();
        try_run(build, builder.tool_cmd(Tool::Linkchecker)
                            .arg(build.out.join(host).join("doc")));
    }

    fn should_run(run: ShouldRun) -> ShouldRun {
        run.path("src/tools/linkchecker")
    }

    fn make_run(
        builder: &Builder,
        path: Option<&Path>,
        host: Interned<String>,
        _target: Interned<String>,
    ) {
        if path.is_some() {
            builder.ensure(Linkcheck { host });
        } else {
            if builder.build.config.docs {
                builder.ensure(Linkcheck { host });
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Cargotest {
    stage: u32,
    host: Interned<String>,
}

impl Step for Cargotest {
    type Output = ();
    const ONLY_HOSTS: bool = true;

    fn should_run(run: ShouldRun) -> ShouldRun {
        run.path("src/tools/cargotest")
    }

    fn make_run(
        builder: &Builder,
        _path: Option<&Path>,
        host: Interned<String>,
        _target: Interned<String>,
    ) {
        builder.ensure(Cargotest {
            stage: builder.top_stage,
            host: host,
        });
    }

    /// Runs the `cargotest` tool as compiled in `stage` by the `host` compiler.
    ///
    /// This tool in `src/tools` will check out a few Rust projects and run `cargo
    /// test` to ensure that we don't regress the test suites there.
    fn run(self, builder: &Builder) {
        let build = builder.build;
        let compiler = builder.compiler(self.stage, self.host);
        builder.ensure(compile::Rustc { compiler, target: compiler.host });

        // Note that this is a short, cryptic, and not scoped directory name. This
        // is currently to minimize the length of path on Windows where we otherwise
        // quickly run into path name limit constraints.
        let out_dir = build.out.join("ct");
        t!(fs::create_dir_all(&out_dir));

        let _time = util::timeit();
        let mut cmd = builder.tool_cmd(Tool::CargoTest);
        try_run(build, cmd.arg(&build.initial_cargo)
                          .arg(&out_dir)
                          .env("RUSTC", builder.rustc(compiler))
                          .env("RUSTDOC", builder.rustdoc(compiler)));
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Cargo {
    stage: u32,
    host: Interned<String>,
}

impl Step for Cargo {
    type Output = ();
    const ONLY_HOSTS: bool = true;

    fn should_run(run: ShouldRun) -> ShouldRun {
        run.path("src/tools/cargo")
    }

    fn make_run(
        builder: &Builder,
        _path: Option<&Path>,
        _host: Interned<String>,
        target: Interned<String>,
    ) {
        builder.ensure(Cargo {
            stage: builder.top_stage,
            host: target,
        });
    }

    /// Runs `cargo test` for `cargo` packaged with Rust.
    fn run(self, builder: &Builder) {
        let build = builder.build;
        let compiler = builder.compiler(self.stage, self.host);

        builder.ensure(tool::Cargo { stage: self.stage, target: self.host });
        let mut cargo = builder.cargo(compiler, Mode::Tool, self.host, "test");
        cargo.arg("--manifest-path").arg(build.src.join("src/tools/cargo/Cargo.toml"));
        if !build.fail_fast {
            cargo.arg("--no-fail-fast");
        }

        // Don't build tests dynamically, just a pain to work with
        cargo.env("RUSTC_NO_PREFER_DYNAMIC", "1");

        // Don't run cross-compile tests, we may not have cross-compiled libstd libs
        // available.
        cargo.env("CFG_DISABLE_CROSS_TESTS", "1");

        try_run(build, cargo.env("PATH", &path_for_cargo(builder, compiler)));
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Rls {
    stage: u32,
    host: Interned<String>,
}

impl Step for Rls {
    type Output = ();
    const ONLY_HOSTS: bool = true;

    fn should_run(run: ShouldRun) -> ShouldRun {
        run.path("src/tools/rls")
    }

    fn make_run(
        builder: &Builder,
        _path: Option<&Path>,
        _host: Interned<String>,
        target: Interned<String>,
    ) {
        builder.ensure(Rls {
            stage: builder.top_stage,
            host: target,
        });
    }

    /// Runs `cargo test` for the rls.
    fn run(self, builder: &Builder) {
        let build = builder.build;
        let stage = self.stage;
        let host = self.host;
        let compiler = builder.compiler(stage, host);

        builder.ensure(tool::Rls { stage: self.stage, target: self.host });
        let mut cargo = builder.cargo(compiler, Mode::Tool, host, "test");
        cargo.arg("--manifest-path").arg(build.src.join("src/tools/rls/Cargo.toml"));

        // Don't build tests dynamically, just a pain to work with
        cargo.env("RUSTC_NO_PREFER_DYNAMIC", "1");

        builder.add_rustc_lib_path(compiler, &mut cargo);

        try_run(build, &mut cargo);
    }
}

fn path_for_cargo(builder: &Builder, compiler: Compiler) -> OsString {
    // Configure PATH to find the right rustc. NB. we have to use PATH
    // and not RUSTC because the Cargo test suite has tests that will
    // fail if rustc is not spelled `rustc`.
    let path = builder.sysroot(compiler).join("bin");
    let old_path = env::var_os("PATH").unwrap_or_default();
    env::join_paths(iter::once(path).chain(env::split_paths(&old_path))).expect("")
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Tidy {
    host: Interned<String>,
}

impl Step for Tidy {
    type Output = ();
    const DEFAULT: bool = true;
    const ONLY_HOSTS: bool = true;
    const ONLY_BUILD: bool = true;

    /// Runs the `tidy` tool as compiled in `stage` by the `host` compiler.
    ///
    /// This tool in `src/tools` checks up on various bits and pieces of style and
    /// otherwise just implements a few lint-like checks that are specific to the
    /// compiler itself.
    fn run(self, builder: &Builder) {
        let build = builder.build;
        let host = self.host;

        let _folder = build.fold_output(|| "tidy");
        println!("tidy check ({})", host);
        let mut cmd = builder.tool_cmd(Tool::Tidy);
        cmd.arg(build.src.join("src"));
        if !build.config.vendor {
            cmd.arg("--no-vendor");
        }
        if build.config.quiet_tests {
            cmd.arg("--quiet");
        }
        try_run(build, &mut cmd);
    }

    fn should_run(run: ShouldRun) -> ShouldRun {
        run.path("src/tools/tidy")
    }

    fn make_run(
        builder: &Builder,
        _path: Option<&Path>,
        _host: Interned<String>,
        _target: Interned<String>,
    ) {
        builder.ensure(Tidy {
            host: builder.build.build,
        });
    }
}

fn testdir(build: &Build, host: Interned<String>) -> PathBuf {
    build.out.join(host).join("test")
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
struct Test {
    path: &'static str,
    mode: &'static str,
    suite: &'static str,
}

static DEFAULT_COMPILETESTS: &[Test] = &[
    Test { path: "src/test/ui", mode: "ui", suite: "ui" },
    Test { path: "src/test/run-pass", mode: "run-pass", suite: "run-pass" },
    Test { path: "src/test/compile-fail", mode: "compile-fail", suite: "compile-fail" },
    Test { path: "src/test/parse-fail", mode: "parse-fail", suite: "parse-fail" },
    Test { path: "src/test/run-fail", mode: "run-fail", suite: "run-fail" },
    Test {
        path: "src/test/run-pass-valgrind",
        mode: "run-pass-valgrind",
        suite: "run-pass-valgrind"
    },
    Test { path: "src/test/mir-opt", mode: "mir-opt", suite: "mir-opt" },
    Test { path: "src/test/codegen", mode: "codegen", suite: "codegen" },
    Test { path: "src/test/codegen-units", mode: "codegen-units", suite: "codegen-units" },
    Test { path: "src/test/incremental", mode: "incremental", suite: "incremental" },

    // What this runs varies depending on the native platform being apple
    Test { path: "src/test/debuginfo", mode: "debuginfo-XXX", suite: "debuginfo" },
];

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct DefaultCompiletest {
    compiler: Compiler,
    target: Interned<String>,
    mode: &'static str,
    suite: &'static str,
}

impl Step for DefaultCompiletest {
    type Output = ();
    const DEFAULT: bool = true;

    fn should_run(mut run: ShouldRun) -> ShouldRun {
        for test in DEFAULT_COMPILETESTS {
            run = run.path(test.path);
        }
        run
    }

    fn make_run(
        builder: &Builder,
        path: Option<&Path>,
        host: Interned<String>,
        target: Interned<String>,
    ) {
        let compiler = builder.compiler(builder.top_stage, host);

        let test = path.map(|path| {
            DEFAULT_COMPILETESTS.iter().find(|&&test| {
                path.ends_with(test.path)
            }).unwrap_or_else(|| {
                panic!("make_run in compile test to receive test path, received {:?}", path);
            })
        });

        if let Some(test) = test {
            builder.ensure(DefaultCompiletest {
                compiler,
                target,
                mode: test.mode,
                suite: test.suite,
            });
        } else {
            for test in DEFAULT_COMPILETESTS {
                builder.ensure(DefaultCompiletest {
                    compiler,
                    target,
                    mode: test.mode,
                    suite: test.suite
                });
            }
        }
    }

    fn run(self, builder: &Builder) {
        builder.ensure(Compiletest {
            compiler: self.compiler,
            target: self.target,
            mode: self.mode,
            suite: self.suite,
        })
    }
}

// Also default, but host-only.
static HOST_COMPILETESTS: &[Test] = &[
    Test { path: "src/test/ui-fulldeps", mode: "ui", suite: "ui-fulldeps" },
    Test { path: "src/test/run-pass-fulldeps", mode: "run-pass", suite: "run-pass-fulldeps" },
    Test { path: "src/test/run-fail-fulldeps", mode: "run-fail", suite: "run-fail-fulldeps" },
    Test {
        path: "src/test/compile-fail-fulldeps",
        mode: "compile-fail",
        suite: "compile-fail-fulldeps",
    },
    Test { path: "src/test/run-make", mode: "run-make", suite: "run-make" },
    Test { path: "src/test/rustdoc", mode: "rustdoc", suite: "rustdoc" },

    Test { path: "src/test/pretty", mode: "pretty", suite: "pretty" },
    Test { path: "src/test/run-pass/pretty", mode: "pretty", suite: "run-pass" },
    Test { path: "src/test/run-fail/pretty", mode: "pretty", suite: "run-fail" },
    Test { path: "src/test/run-pass-valgrind/pretty", mode: "pretty", suite: "run-pass-valgrind" },
    Test { path: "src/test/run-pass-fulldeps/pretty", mode: "pretty", suite: "run-pass-fulldeps" },
    Test { path: "src/test/run-fail-fulldeps/pretty", mode: "pretty", suite: "run-fail-fulldeps" },
];

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct HostCompiletest {
    compiler: Compiler,
    target: Interned<String>,
    mode: &'static str,
    suite: &'static str,
}

impl Step for HostCompiletest {
    type Output = ();
    const DEFAULT: bool = true;
    const ONLY_HOSTS: bool = true;

    fn should_run(mut run: ShouldRun) -> ShouldRun {
        for test in HOST_COMPILETESTS {
            run = run.path(test.path);
        }
        run
    }

    fn make_run(
        builder: &Builder,
        path: Option<&Path>,
        host: Interned<String>,
        target: Interned<String>,
    ) {
        let compiler = builder.compiler(builder.top_stage, host);

        let test = path.map(|path| {
            HOST_COMPILETESTS.iter().find(|&&test| {
                path.ends_with(test.path)
            }).unwrap_or_else(|| {
                panic!("make_run in compile test to receive test path, received {:?}", path);
            })
        });

        if let Some(test) = test {
            builder.ensure(HostCompiletest {
                compiler,
                target,
                mode: test.mode,
                suite: test.suite,
            });
        } else {
            for test in HOST_COMPILETESTS {
                builder.ensure(HostCompiletest {
                    compiler,
                    target,
                    mode: test.mode,
                    suite: test.suite
                });
            }
        }
    }

    fn run(self, builder: &Builder) {
        builder.ensure(Compiletest {
            compiler: self.compiler,
            target: self.target,
            mode: self.mode,
            suite: self.suite,
        })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
struct Compiletest {
    compiler: Compiler,
    target: Interned<String>,
    mode: &'static str,
    suite: &'static str,
}

impl Step for Compiletest {
    type Output = ();

    fn should_run(run: ShouldRun) -> ShouldRun {
        run.never()
    }

    /// Executes the `compiletest` tool to run a suite of tests.
    ///
    /// Compiles all tests with `compiler` for `target` with the specified
    /// compiletest `mode` and `suite` arguments. For example `mode` can be
    /// "run-pass" or `suite` can be something like `debuginfo`.
    fn run(self, builder: &Builder) {
        let build = builder.build;
        let compiler = self.compiler;
        let target = self.target;
        let mode = self.mode;
        let suite = self.suite;

        // Skip codegen tests if they aren't enabled in configuration.
        if !build.config.codegen_tests && suite == "codegen" {
            return;
        }

        if suite == "debuginfo" {
            // Skip debuginfo tests on MSVC
            if build.build.contains("msvc") {
                return;
            }

            if mode == "debuginfo-XXX" {
                return if build.build.contains("apple") {
                    builder.ensure(Compiletest {
                        mode: "debuginfo-lldb",
                        ..self
                    });
                } else {
                    builder.ensure(Compiletest {
                        mode: "debuginfo-gdb",
                        ..self
                    });
                };
            }

            builder.ensure(dist::DebuggerScripts {
                sysroot: builder.sysroot(compiler),
                target: target
            });
        }

        if suite.ends_with("fulldeps") ||
            // FIXME: Does pretty need librustc compiled? Note that there are
            // fulldeps test suites with mode = pretty as well.
            mode == "pretty" ||
            mode == "rustdoc" ||
            mode == "run-make" {
            builder.ensure(compile::Rustc { compiler, target });
        }

        builder.ensure(compile::Test { compiler, target });
        builder.ensure(native::TestHelpers { target });
        builder.ensure(RemoteCopyLibs { compiler, target });

        let _folder = build.fold_output(|| format!("test_{}", suite));
        println!("Check compiletest suite={} mode={} ({} -> {})",
                 suite, mode, &compiler.host, target);
        let mut cmd = builder.tool_cmd(Tool::Compiletest);

        // compiletest currently has... a lot of arguments, so let's just pass all
        // of them!

        cmd.arg("--compile-lib-path").arg(builder.rustc_libdir(compiler));
        cmd.arg("--run-lib-path").arg(builder.sysroot_libdir(compiler, target));
        cmd.arg("--rustc-path").arg(builder.rustc(compiler));
        cmd.arg("--rustdoc-path").arg(builder.rustdoc(compiler));
        cmd.arg("--src-base").arg(build.src.join("src/test").join(suite));
        cmd.arg("--build-base").arg(testdir(build, compiler.host).join(suite));
        cmd.arg("--stage-id").arg(format!("stage{}-{}", compiler.stage, target));
        cmd.arg("--mode").arg(mode);
        cmd.arg("--target").arg(target);
        cmd.arg("--host").arg(&*compiler.host);
        cmd.arg("--llvm-filecheck").arg(build.llvm_filecheck(build.build));

        if let Some(ref nodejs) = build.config.nodejs {
            cmd.arg("--nodejs").arg(nodejs);
        }

        let mut flags = vec!["-Crpath".to_string()];
        if build.config.rust_optimize_tests {
            flags.push("-O".to_string());
        }
        if build.config.rust_debuginfo_tests {
            flags.push("-g".to_string());
        }

        let mut hostflags = build.rustc_flags(compiler.host);
        hostflags.extend(flags.clone());
        cmd.arg("--host-rustcflags").arg(hostflags.join(" "));

        let mut targetflags = build.rustc_flags(target);
        targetflags.extend(flags);
        targetflags.push(format!("-Lnative={}",
                                 build.test_helpers_out(target).display()));
        cmd.arg("--target-rustcflags").arg(targetflags.join(" "));

        cmd.arg("--docck-python").arg(build.python());

        if build.build.ends_with("apple-darwin") {
            // Force /usr/bin/python on macOS for LLDB tests because we're loading the
            // LLDB plugin's compiled module which only works with the system python
            // (namely not Homebrew-installed python)
            cmd.arg("--lldb-python").arg("/usr/bin/python");
        } else {
            cmd.arg("--lldb-python").arg(build.python());
        }

        if let Some(ref gdb) = build.config.gdb {
            cmd.arg("--gdb").arg(gdb);
        }
        if let Some(ref vers) = build.lldb_version {
            cmd.arg("--lldb-version").arg(vers);
        }
        if let Some(ref dir) = build.lldb_python_dir {
            cmd.arg("--lldb-python-dir").arg(dir);
        }
        let llvm_config = build.llvm_config(target);
        let llvm_version = output(Command::new(&llvm_config).arg("--version"));
        cmd.arg("--llvm-version").arg(llvm_version);
        if !build.is_rust_llvm(target) {
            cmd.arg("--system-llvm");
        }

        cmd.args(&build.flags.cmd.test_args());

        if build.is_verbose() {
            cmd.arg("--verbose");
        }

        if build.config.quiet_tests {
            cmd.arg("--quiet");
        }

        // Only pass correct values for these flags for the `run-make` suite as it
        // requires that a C++ compiler was configured which isn't always the case.
        if suite == "run-make" {
            let llvm_components = output(Command::new(&llvm_config).arg("--components"));
            let llvm_cxxflags = output(Command::new(&llvm_config).arg("--cxxflags"));
            cmd.arg("--cc").arg(build.cc(target))
               .arg("--cxx").arg(build.cxx(target).unwrap())
               .arg("--cflags").arg(build.cflags(target).join(" "))
               .arg("--llvm-components").arg(llvm_components.trim())
               .arg("--llvm-cxxflags").arg(llvm_cxxflags.trim());
        } else {
            cmd.arg("--cc").arg("")
               .arg("--cxx").arg("")
               .arg("--cflags").arg("")
               .arg("--llvm-components").arg("")
               .arg("--llvm-cxxflags").arg("");
        }

        if build.remote_tested(target) {
            cmd.arg("--remote-test-client").arg(builder.tool_exe(Tool::RemoteTestClient));
        }

        // Running a C compiler on MSVC requires a few env vars to be set, to be
        // sure to set them here.
        //
        // Note that if we encounter `PATH` we make sure to append to our own `PATH`
        // rather than stomp over it.
        if target.contains("msvc") {
            for &(ref k, ref v) in build.cc[&target].0.env() {
                if k != "PATH" {
                    cmd.env(k, v);
                }
            }
        }
        cmd.env("RUSTC_BOOTSTRAP", "1");
        build.add_rust_test_threads(&mut cmd);

        if build.config.sanitizers {
            cmd.env("SANITIZER_SUPPORT", "1");
        }

        if build.config.profiler {
            cmd.env("PROFILER_SUPPORT", "1");
        }

        cmd.arg("--adb-path").arg("adb");
        cmd.arg("--adb-test-dir").arg(ADB_TEST_DIR);
        if target.contains("android") {
            // Assume that cc for this target comes from the android sysroot
            cmd.arg("--android-cross-path")
               .arg(build.cc(target).parent().unwrap().parent().unwrap());
        } else {
            cmd.arg("--android-cross-path").arg("");
        }

        build.ci_env.force_coloring_in_ci(&mut cmd);

        let _time = util::timeit();
        try_run(build, &mut cmd);
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Docs {
    compiler: Compiler,
}

impl Step for Docs {
    type Output = ();
    const DEFAULT: bool = true;
    const ONLY_HOSTS: bool = true;

    fn should_run(run: ShouldRun) -> ShouldRun {
        run.path("src/doc")
    }

    fn make_run(
        builder: &Builder,
        _path: Option<&Path>,
        host: Interned<String>,
        _target: Interned<String>,
    ) {
        builder.ensure(Docs {
            compiler: builder.compiler(builder.top_stage, host),
        });
    }

    /// Run `rustdoc --test` for all documentation in `src/doc`.
    ///
    /// This will run all tests in our markdown documentation (e.g. the book)
    /// located in `src/doc`. The `rustdoc` that's run is the one that sits next to
    /// `compiler`.
    fn run(self, builder: &Builder) {
        let build = builder.build;
        let compiler = self.compiler;

        builder.ensure(compile::Test { compiler, target: compiler.host });

        // Do a breadth-first traversal of the `src/doc` directory and just run
        // tests for all files that end in `*.md`
        let mut stack = vec![build.src.join("src/doc")];
        let _time = util::timeit();
        let _folder = build.fold_output(|| "test_docs");

        while let Some(p) = stack.pop() {
            if p.is_dir() {
                stack.extend(t!(p.read_dir()).map(|p| t!(p).path()));
                continue
            }

            if p.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }

            // The nostarch directory in the book is for no starch, and so isn't
            // guaranteed to build. We don't care if it doesn't build, so skip it.
            if p.to_str().map_or(false, |p| p.contains("nostarch")) {
                continue;
            }

            markdown_test(builder, compiler, &p);
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct ErrorIndex {
    compiler: Compiler,
}

impl Step for ErrorIndex {
    type Output = ();
    const DEFAULT: bool = true;
    const ONLY_HOSTS: bool = true;

    fn should_run(run: ShouldRun) -> ShouldRun {
        run.path("src/tools/error_index_generator")
    }

    fn make_run(
        builder: &Builder,
        _path: Option<&Path>,
        host: Interned<String>,
        _target: Interned<String>,
    ) {
        builder.ensure(ErrorIndex {
            compiler: builder.compiler(builder.top_stage, host),
        });
    }

    /// Run the error index generator tool to execute the tests located in the error
    /// index.
    ///
    /// The `error_index_generator` tool lives in `src/tools` and is used to
    /// generate a markdown file from the error indexes of the code base which is
    /// then passed to `rustdoc --test`.
    fn run(self, builder: &Builder) {
        let build = builder.build;
        let compiler = self.compiler;

        builder.ensure(compile::Std { compiler, target: compiler.host });

        let _folder = build.fold_output(|| "test_error_index");
        println!("Testing error-index stage{}", compiler.stage);

        let dir = testdir(build, compiler.host);
        t!(fs::create_dir_all(&dir));
        let output = dir.join("error-index.md");

        let _time = util::timeit();
        build.run(builder.tool_cmd(Tool::ErrorIndex)
                    .arg("markdown")
                    .arg(&output)
                    .env("CFG_BUILD", &build.build));

        markdown_test(builder, compiler, &output);
    }
}

fn markdown_test(builder: &Builder, compiler: Compiler, markdown: &Path) {
    let build = builder.build;
    let mut file = t!(File::open(markdown));
    let mut contents = String::new();
    t!(file.read_to_string(&mut contents));
    if !contents.contains("```") {
        return;
    }

    println!("doc tests for: {}", markdown.display());
    let mut cmd = Command::new(builder.rustdoc(compiler));
    builder.add_rustc_lib_path(compiler, &mut cmd);
    build.add_rust_test_threads(&mut cmd);
    cmd.arg("--test");
    cmd.arg(markdown);
    cmd.env("RUSTC_BOOTSTRAP", "1");

    let test_args = build.flags.cmd.test_args().join(" ");
    cmd.arg("--test-args").arg(test_args);

    if build.config.quiet_tests {
        try_run_quiet(build, &mut cmd);
    } else {
        try_run(build, &mut cmd);
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct CrateLibrustc {
    compiler: Compiler,
    target: Interned<String>,
    test_kind: TestKind,
    krate: Option<Interned<String>>,
}

impl Step for CrateLibrustc {
    type Output = ();
    const DEFAULT: bool = true;
    const ONLY_HOSTS: bool = true;

    fn should_run(run: ShouldRun) -> ShouldRun {
        run.krate("rustc-main")
    }

    fn make_run(
        builder: &Builder,
        path: Option<&Path>,
        host: Interned<String>,
        target: Interned<String>,
    ) {
        let compiler = builder.compiler(builder.top_stage, host);

        let run = |name: Option<Interned<String>>| {
            let test_kind = if builder.kind == Kind::Test {
                TestKind::Test
            } else if builder.kind == Kind::Bench {
                TestKind::Bench
            } else {
                panic!("unexpected builder.kind in crate: {:?}", builder.kind);
            };

            builder.ensure(CrateLibrustc {
                compiler,
                target,
                test_kind: test_kind,
                krate: name,
            });
        };

        if let Some(path) = path {
            for (name, krate_path) in builder.crates("rustc-main") {
                if path.ends_with(krate_path) {
                    run(Some(name));
                }
            }
        } else {
            run(None);
        }
    }


    fn run(self, builder: &Builder) {
        builder.ensure(Crate {
            compiler: self.compiler,
            target: self.target,
            mode: Mode::Librustc,
            test_kind: self.test_kind,
            krate: self.krate,
        });
    }
}


#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Crate {
    compiler: Compiler,
    target: Interned<String>,
    mode: Mode,
    test_kind: TestKind,
    krate: Option<Interned<String>>,
}

impl Step for Crate {
    type Output = ();
    const DEFAULT: bool = true;

    fn should_run(run: ShouldRun) -> ShouldRun {
        run.krate("std").krate("test")
    }

    fn make_run(
        builder: &Builder,
        path: Option<&Path>,
        host: Interned<String>,
        target: Interned<String>,
    ) {
        let compiler = builder.compiler(builder.top_stage, host);

        let run = |mode: Mode, name: Option<Interned<String>>| {
            let test_kind = if builder.kind == Kind::Test {
                TestKind::Test
            } else if builder.kind == Kind::Bench {
                TestKind::Bench
            } else {
                panic!("unexpected builder.kind in crate: {:?}", builder.kind);
            };

            builder.ensure(Crate {
                compiler, target,
                mode: mode,
                test_kind: test_kind,
                krate: name,
            });
        };

        if let Some(path) = path {
            for (name, krate_path) in builder.crates("std") {
                if path.ends_with(krate_path) {
                    run(Mode::Libstd, Some(name));
                }
            }
            for (name, krate_path) in builder.crates("test") {
                if path.ends_with(krate_path) {
                    run(Mode::Libtest, Some(name));
                }
            }
        } else {
            run(Mode::Libstd, None);
            run(Mode::Libtest, None);
        }
    }

    /// Run all unit tests plus documentation tests for an entire crate DAG defined
    /// by a `Cargo.toml`
    ///
    /// This is what runs tests for crates like the standard library, compiler, etc.
    /// It essentially is the driver for running `cargo test`.
    ///
    /// Currently this runs all tests for a DAG by passing a bunch of `-p foo`
    /// arguments, and those arguments are discovered from `cargo metadata`.
    fn run(self, builder: &Builder) {
        let build = builder.build;
        let compiler = self.compiler;
        let target = self.target;
        let mode = self.mode;
        let test_kind = self.test_kind;
        let krate = self.krate;

        builder.ensure(compile::Test { compiler, target });
        builder.ensure(RemoteCopyLibs { compiler, target });
        let (name, path, features, root) = match mode {
            Mode::Libstd => {
                ("libstd", "src/libstd", build.std_features(), "std")
            }
            Mode::Libtest => {
                ("libtest", "src/libtest", String::new(), "test")
            }
            Mode::Librustc => {
                builder.ensure(compile::Rustc { compiler, target });
                ("librustc", "src/rustc", build.rustc_features(), "rustc-main")
            }
            _ => panic!("can only test libraries"),
        };
        let root = INTERNER.intern_string(String::from(root));
        let _folder = build.fold_output(|| {
            format!("{}_stage{}-{}", test_kind.subcommand(), compiler.stage, name)
        });
        println!("{} {} stage{} ({} -> {})", test_kind, name, compiler.stage,
                &compiler.host, target);

        // If we're not doing a full bootstrap but we're testing a stage2 version of
        // libstd, then what we're actually testing is the libstd produced in
        // stage1. Reflect that here by updating the compiler that we're working
        // with automatically.
        let compiler = if build.force_use_stage1(compiler, target) {
            builder.compiler(1, compiler.host)
        } else {
            compiler.clone()
        };

        // Build up the base `cargo test` command.
        //
        // Pass in some standard flags then iterate over the graph we've discovered
        // in `cargo metadata` with the maps above and figure out what `-p`
        // arguments need to get passed.
        let mut cargo = builder.cargo(compiler, mode, target, test_kind.subcommand());
        cargo.arg("--manifest-path")
            .arg(build.src.join(path).join("Cargo.toml"))
            .arg("--features").arg(features);
        if test_kind.subcommand() == "test" && !build.fail_fast {
            cargo.arg("--no-fail-fast");
        }

        match krate {
            Some(krate) => {
                cargo.arg("-p").arg(krate);
            }
            None => {
                let mut visited = HashSet::new();
                let mut next = vec![root];
                while let Some(name) = next.pop() {
                    // Right now jemalloc is our only target-specific crate in the
                    // sense that it's not present on all platforms. Custom skip it
                    // here for now, but if we add more this probably wants to get
                    // more generalized.
                    //
                    // Also skip `build_helper` as it's not compiled normally for
                    // target during the bootstrap and it's just meant to be a
                    // helper crate, not tested. If it leaks through then it ends up
                    // messing with various mtime calculations and such.
                    if !name.contains("jemalloc") && *name != *"build_helper" {
                        cargo.arg("-p").arg(&format!("{}:0.0.0", name));
                    }
                    for dep in build.crates[&name].deps.iter() {
                        if visited.insert(dep) {
                            next.push(*dep);
                        }
                    }
                }
            }
        }

        // The tests are going to run with the *target* libraries, so we need to
        // ensure that those libraries show up in the LD_LIBRARY_PATH equivalent.
        //
        // Note that to run the compiler we need to run with the *host* libraries,
        // but our wrapper scripts arrange for that to be the case anyway.
        let mut dylib_path = dylib_path();
        dylib_path.insert(0, PathBuf::from(&*builder.sysroot_libdir(compiler, target)));
        cargo.env(dylib_path_var(), env::join_paths(&dylib_path).unwrap());

        if target.contains("emscripten") || build.remote_tested(target) {
            cargo.arg("--no-run");
        }

        cargo.arg("--");

        if build.config.quiet_tests {
            cargo.arg("--quiet");
        }

        let _time = util::timeit();

        if target.contains("emscripten") {
            build.run(&mut cargo);
            krate_emscripten(build, compiler, target, mode);
        } else if build.remote_tested(target) {
            build.run(&mut cargo);
            krate_remote(builder, compiler, target, mode);
        } else {
            cargo.args(&build.flags.cmd.test_args());
            try_run(build, &mut cargo);
        }
    }
}

fn krate_emscripten(build: &Build,
                    compiler: Compiler,
                    target: Interned<String>,
                    mode: Mode) {
    let out_dir = build.cargo_out(compiler, mode, target);
    let tests = find_tests(&out_dir.join("deps"), target);

    let nodejs = build.config.nodejs.as_ref().expect("nodejs not configured");
    for test in tests {
        println!("running {}", test.display());
        let mut cmd = Command::new(nodejs);
        cmd.arg(&test);
        if build.config.quiet_tests {
            cmd.arg("--quiet");
        }
        try_run(build, &mut cmd);
    }
}

fn krate_remote(builder: &Builder,
                compiler: Compiler,
                target: Interned<String>,
                mode: Mode) {
    let build = builder.build;
    let out_dir = build.cargo_out(compiler, mode, target);
    let tests = find_tests(&out_dir.join("deps"), target);

    let tool = builder.tool_exe(Tool::RemoteTestClient);
    for test in tests {
        let mut cmd = Command::new(&tool);
        cmd.arg("run")
           .arg(&test);
        if build.config.quiet_tests {
            cmd.arg("--quiet");
        }
        cmd.args(&build.flags.cmd.test_args());
        try_run(build, &mut cmd);
    }
}

fn find_tests(dir: &Path, target: Interned<String>) -> Vec<PathBuf> {
    let mut dst = Vec::new();
    for e in t!(dir.read_dir()).map(|e| t!(e)) {
        let file_type = t!(e.file_type());
        if !file_type.is_file() {
            continue
        }
        let filename = e.file_name().into_string().unwrap();
        if (target.contains("windows") && filename.ends_with(".exe")) ||
           (!target.contains("windows") && !filename.contains(".")) ||
           (target.contains("emscripten") &&
            filename.ends_with(".js") &&
            !filename.ends_with(".asm.js")) {
            dst.push(e.path());
        }
    }
    dst
}

/// Some test suites are run inside emulators or on remote devices, and most
/// of our test binaries are linked dynamically which means we need to ship
/// the standard library and such to the emulator ahead of time. This step
/// represents this and is a dependency of all test suites.
///
/// Most of the time this is a noop. For some steps such as shipping data to
/// QEMU we have to build our own tools so we've got conditional dependencies
/// on those programs as well. Note that the remote test client is built for
/// the build target (us) and the server is built for the target.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct RemoteCopyLibs {
    compiler: Compiler,
    target: Interned<String>,
}

impl Step for RemoteCopyLibs {
    type Output = ();

    fn should_run(run: ShouldRun) -> ShouldRun {
        run.never()
    }

    fn run(self, builder: &Builder) {
        let build = builder.build;
        let compiler = self.compiler;
        let target = self.target;
        if !build.remote_tested(target) {
            return
        }

        builder.ensure(compile::Test { compiler, target });

        println!("REMOTE copy libs to emulator ({})", target);
        t!(fs::create_dir_all(build.out.join("tmp")));

        let server = builder.ensure(tool::RemoteTestServer { stage: compiler.stage, target });

        // Spawn the emulator and wait for it to come online
        let tool = builder.tool_exe(Tool::RemoteTestClient);
        let mut cmd = Command::new(&tool);
        cmd.arg("spawn-emulator")
           .arg(target)
           .arg(&server)
           .arg(build.out.join("tmp"));
        if let Some(rootfs) = build.qemu_rootfs(target) {
            cmd.arg(rootfs);
        }
        build.run(&mut cmd);

        // Push all our dylibs to the emulator
        for f in t!(builder.sysroot_libdir(compiler, target).read_dir()) {
            let f = t!(f);
            let name = f.file_name().into_string().unwrap();
            if util::is_dylib(&name) {
                build.run(Command::new(&tool)
                                  .arg("push")
                                  .arg(f.path()));
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Distcheck;

impl Step for Distcheck {
    type Output = ();

    fn should_run(run: ShouldRun) -> ShouldRun {
        run.path("distcheck")
    }

    /// Run "distcheck", a 'make check' from a tarball
    fn run(self, builder: &Builder) {
        let build = builder.build;

        if *build.build != *"x86_64-unknown-linux-gnu" {
            return
        }
        if !build.config.host.iter().any(|s| s == "x86_64-unknown-linux-gnu") {
            return
        }
        if !build.config.target.iter().any(|s| s == "x86_64-unknown-linux-gnu") {
            return
        }

        println!("Distcheck");
        let dir = build.out.join("tmp").join("distcheck");
        let _ = fs::remove_dir_all(&dir);
        t!(fs::create_dir_all(&dir));

        let mut cmd = Command::new("tar");
        cmd.arg("-xzf")
           .arg(builder.ensure(dist::PlainSourceTarball))
           .arg("--strip-components=1")
           .current_dir(&dir);
        build.run(&mut cmd);
        build.run(Command::new("./configure")
                         .args(&build.config.configure_args)
                         .arg("--enable-vendor")
                         .current_dir(&dir));
        build.run(Command::new(build_helper::make(&build.build))
                         .arg("check")
                         .current_dir(&dir));

        // Now make sure that rust-src has all of libstd's dependencies
        println!("Distcheck rust-src");
        let dir = build.out.join("tmp").join("distcheck-src");
        let _ = fs::remove_dir_all(&dir);
        t!(fs::create_dir_all(&dir));

        let mut cmd = Command::new("tar");
        cmd.arg("-xzf")
           .arg(builder.ensure(dist::Src))
           .arg("--strip-components=1")
           .current_dir(&dir);
        build.run(&mut cmd);

        let toml = dir.join("rust-src/lib/rustlib/src/rust/src/libstd/Cargo.toml");
        build.run(Command::new(&build.initial_cargo)
                         .arg("generate-lockfile")
                         .arg("--manifest-path")
                         .arg(&toml)
                         .current_dir(&dir));
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Bootstrap;

impl Step for Bootstrap {
    type Output = ();
    const DEFAULT: bool = true;
    const ONLY_HOSTS: bool = true;
    const ONLY_BUILD: bool = true;

    /// Test the build system itself
    fn run(self, builder: &Builder) {
        let build = builder.build;
        let mut cmd = Command::new(&build.initial_cargo);
        cmd.arg("test")
           .current_dir(build.src.join("src/bootstrap"))
           .env("CARGO_TARGET_DIR", build.out.join("bootstrap"))
           .env("RUSTC_BOOTSTRAP", "1")
           .env("RUSTC", &build.initial_rustc);
        if !build.fail_fast {
            cmd.arg("--no-fail-fast");
        }
        cmd.arg("--").args(&build.flags.cmd.test_args());
        try_run(build, &mut cmd);
    }

    fn should_run(run: ShouldRun) -> ShouldRun {
        run.path("src/bootstrap")
    }

    fn make_run(
        builder: &Builder,
        _path: Option<&Path>,
        _host: Interned<String>,
        _target: Interned<String>,
    ) {
        builder.ensure(Bootstrap);
    }
}
