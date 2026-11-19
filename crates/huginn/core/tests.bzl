load("@crates//:defs.bzl", "all_crate_deps", "crate_edition")
load("@rules_rust//rust:defs.bzl", "rust_test")


def huginn_core_integration_tests(srcs, support_srcs):
    for src in srcs:
        name = src.split("/")[-1].removesuffix(".rs")
        rust_test(
            name = name,
            crate_name = name,
            edition = crate_edition(),
            srcs = [src] + ([support_srcs] if src.endswith("tool_hook_evaluation.rs") or src.endswith("tool_hooks.rs") else []),
            deps = ["//crates/huginn/core:huginn_core"] + all_crate_deps(normal = True, normal_dev = True),
            proc_macro_deps = all_crate_deps(proc_macro = True, proc_macro_dev = True),
        )
