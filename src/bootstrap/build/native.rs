// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Compilation of native dependencies like LLVM.
//!
//! Native projects like LLVM unfortunately aren't suited just yet for
//! compilation in build scripts that Cargo has. This is because thie
//! compilation takes a *very* long time but also because we don't want to
//! compile LLVM 3 times as part of a normal bootstrap (we want it cached).
//!
//! LLVM and compiler-rt are essentially just wired up to everything else to
//! ensure that they're always in place if needed.

use std::path::Path;
use std::process::Command;
use std::fs::{self, File};

use build_helper::output;
use cmake;
use gcc;

use build::Build;
use build::util::{staticlib, up_to_date};

/// Compile LLVM for `target`.
pub fn llvm(build: &Build, target: &str) {
    // If we're using a custom LLVM bail out here, but we can only use a
    // custom LLVM for the build triple.
    if let Some(config) = build.config.target_config.get(target) {
        if let Some(ref s) = config.llvm_config {
            return check_llvm_version(build, s);
        }
    }

    // If the cleaning trigger is newer than our built artifacts (or if the
    // artifacts are missing) then we keep going, otherwise we bail out.
    let dst = build.llvm_out(target);
    let stamp = build.src.join("src/rustllvm/llvm-auto-clean-trigger");
    let done_stamp = dst.join("llvm-finished-building");
    build.clear_if_dirty(&dst, &stamp);
    if fs::metadata(&done_stamp).is_ok() {
        return
    }

    println!("Building LLVM for {}", target);

    let _ = fs::remove_dir_all(&dst.join("build"));
    t!(fs::create_dir_all(&dst.join("build")));
    let assertions = if build.config.llvm_assertions {"ON"} else {"OFF"};

    // http://llvm.org/docs/CMake.html
    let mut cfg = cmake::Config::new(build.src.join("src/llvm"));
    if build.config.ninja {
        cfg.generator("Ninja");
    }
    cfg.target(target)
       .host(&build.config.build)
       .out_dir(&dst)
       .profile(if build.config.llvm_optimize {"Release"} else {"Debug"})
       .define("LLVM_ENABLE_ASSERTIONS", assertions)
       .define("LLVM_TARGETS_TO_BUILD", "X86;ARM;AArch64;Mips;PowerPC")
       .define("LLVM_INCLUDE_EXAMPLES", "OFF")
       .define("LLVM_INCLUDE_TESTS", "OFF")
       .define("LLVM_INCLUDE_DOCS", "OFF")
       .define("LLVM_ENABLE_ZLIB", "OFF")
       .define("WITH_POLLY", "OFF")
       .define("LLVM_ENABLE_TERMINFO", "OFF")
       .define("LLVM_ENABLE_LIBEDIT", "OFF")
       .define("LLVM_PARALLEL_COMPILE_JOBS", build.jobs().to_string());

    if target.starts_with("i686") {
        cfg.define("LLVM_BUILD_32_BITS", "ON");
    }

    // http://llvm.org/docs/HowToCrossCompileLLVM.html
    if target != build.config.build {
        // FIXME: if the llvm root for the build triple is overridden then we
        //        should use llvm-tblgen from there, also should verify that it
        //        actually exists most of the time in normal installs of LLVM.
        let host = build.llvm_out(&build.config.build).join("bin/llvm-tblgen");
        cfg.define("CMAKE_CROSSCOMPILING", "True")
           .define("LLVM_TARGET_ARCH", target.split('-').next().unwrap())
           .define("LLVM_TABLEGEN", &host)
           .define("LLVM_DEFAULT_TARGET_TRIPLE", target);
    }

    // MSVC handles compiler business itself
    if !target.contains("msvc") {
        if build.config.ccache {
           cfg.define("CMAKE_C_COMPILER", "ccache")
              .define("CMAKE_C_COMPILER_ARG1", build.cc(target))
              .define("CMAKE_CXX_COMPILER", "ccache")
              .define("CMAKE_CXX_COMPILER_ARG1", build.cxx(target));
        } else {
           cfg.define("CMAKE_C_COMPILER", build.cc(target))
              .define("CMAKE_CXX_COMPILER", build.cxx(target));
        }
        cfg.build_arg("-j").build_arg(build.jobs().to_string());

        cfg.define("CMAKE_C_FLAGS", build.cflags(target).join(" "));
        cfg.define("CMAKE_CXX_FLAGS", build.cflags(target).join(" "));
    }

    // FIXME: we don't actually need to build all LLVM tools and all LLVM
    //        libraries here, e.g. we just want a few components and a few
    //        tools. Figure out how to filter them down and only build the right
    //        tools and libs on all platforms.
    cfg.build();

    t!(File::create(&done_stamp));
}

fn check_llvm_version(build: &Build, llvm_config: &Path) {
    if !build.config.llvm_version_check {
        return
    }

    let mut cmd = Command::new(llvm_config);
    let version = output(cmd.arg("--version"));
    if version.starts_with("3.5") || version.starts_with("3.6") ||
       version.starts_with("3.7") {
        return
    }
    panic!("\n\nbad LLVM version: {}, need >=3.5\n\n", version)
}

/// Compiles the `compiler-rt` library, or at least the builtins part of it.
///
/// This uses the CMake build system and an existing LLVM build directory to
/// compile the project.
pub fn compiler_rt(build: &Build, target: &str) {
    let dst = build.compiler_rt_out(target);
    let arch = target.split('-').next().unwrap();
    let mode = if build.config.rust_optimize {"Release"} else {"Debug"};

    let build_llvm_config = build.llvm_config(&build.config.build);
    let mut cfg = cmake::Config::new(build.src.join("src/compiler-rt"));
    cfg.target(target)
       .host(&build.config.build)
       .out_dir(&dst)
       .profile(mode)
       .define("LLVM_CONFIG_PATH", build_llvm_config)
       .define("COMPILER_RT_DEFAULT_TARGET_TRIPLE", target)
       .define("COMPILER_RT_BUILD_SANITIZERS", "OFF")
       .define("COMPILER_RT_BUILD_EMUTLS", "OFF")
       // inform about c/c++ compilers, the c++ compiler isn't actually used but
       // it's needed to get the initial configure to work on all platforms.
       .define("CMAKE_C_COMPILER", build.cc(target))
       .define("CMAKE_CXX_COMPILER", build.cc(target));

    let (dir, build_target, libname) = if target.contains("linux") ||
                                          target.contains("freebsd") ||
                                          target.contains("netbsd") {
        let os_extra = if target.contains("android") && target.contains("arm") {
            "-android"
        } else {
            ""
        };
        let builtins_arch = match arch {
            "i586" => "i386",
            "arm" | "armv7" if target.contains("android") => "armhf",
            "arm" if target.contains("eabihf") => "armhf",
            _ => arch,
        };
        let target = format!("clang_rt.builtins-{}", builtins_arch);
        ("linux".to_string(),
         target.clone(),
         format!("{}{}", target, os_extra))
    } else if target.contains("apple-darwin") {
        let builtins_arch = match arch {
            "i686" => "i386",
            _ => arch,
        };
        let target = format!("clang_rt.builtins_{}_osx", builtins_arch);
        ("builtins".to_string(), target.clone(), target)
    } else if target.contains("apple-ios") {
        cfg.define("COMPILER_RT_ENABLE_IOS", "ON");
        let target = match arch {
            "armv7s" => "hard_pic_armv7em_macho_embedded".to_string(),
            "aarch64" => "builtins_arm64_ios".to_string(),
            _ => format!("hard_pic_{}_macho_embedded", arch),
        };
        ("builtins".to_string(), target.clone(), target)
    } else if target.contains("windows-gnu") {
        let target = format!("clang_rt.builtins-{}", arch);
        ("windows".to_string(), target.clone(), target)
    } else if target.contains("windows-msvc") {
        let builtins_arch = match arch {
            "i586" | "i686" => "i386",
            _ => arch,
        };
        (format!("windows/{}", mode),
         "lib/builtins/builtins".to_string(),
         format!("clang_rt.builtins-{}", builtins_arch))
    } else {
        panic!("can't get os from target: {}", target)
    };
    let output = dst.join("build/lib").join(dir)
                    .join(staticlib(&libname, target));
    build.compiler_rt_built.borrow_mut().insert(target.to_string(),
                                                output.clone());
    if fs::metadata(&output).is_ok() {
        return
    }
    let _ = fs::remove_dir_all(&dst);
    t!(fs::create_dir_all(&dst));
    cfg.build_target(&build_target);
    cfg.build();
}

/// Compiles the `rust_test_helpers.c` library which we used in various
/// `run-pass` test suites for ABI testing.
pub fn test_helpers(build: &Build, target: &str) {
    let dst = build.test_helpers_out(target);
    let src = build.src.join("src/rt/rust_test_helpers.c");
    if up_to_date(&src, &dst.join("librust_test_helpers.a")) {
        return
    }

    println!("Building test helpers");
    t!(fs::create_dir_all(&dst));
    let mut cfg = gcc::Config::new();
    cfg.cargo_metadata(false)
       .out_dir(&dst)
       .target(target)
       .host(&build.config.build)
       .opt_level(0)
       .debug(false)
       .file(build.src.join("src/rt/rust_test_helpers.c"))
       .compile("librust_test_helpers.a");
}
