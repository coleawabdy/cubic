set_project("cubic")

-- Global

set_languages("c++23")

set_policy("build.c++.modules", true)

add_rules("mode.debug", "mode.release")

-- Rules

rule("cpp_sanitizer")
    on_config(function (target)
        -- runtime options for leak sanitizer
        if target:policy("build.sanitizer.leak") then
            local lsan_suppressions_path = path.join("$(projectdir)", "config", "sanitizer", "lsan.supp")
            target:add("runenvs", "LSAN_OPTIONS", "suppressions=" .. lsan_suppressions_path .. ":print_suppressions=0")
        end

        -- runtime options for undefined sanitizer
        if target:policy("build.sanitizer.undefined") then
            target:add("runenvs", "UBSAN_OPTIONS", "print_stacktrace=1:halt_on_error=0:color=auto:print_summary=1")
        end    

        -- fix for ubsan symbols not found with LLVM
        if target:toolchain("llvm") and target:policy("build.sanitizer.undefined") then
            target:add("cxflags", "-fno-sanitize-link-runtime")
            target:add("ldflags", "-fno-sanitize-link-runtime")

            local clang_resource_dir = os.iorunv(target:tool("cc"), {"--print-resource-dir"}):trim()
            
            if target:is_plat("macosx") then
                target:add("links", path.join(clang_resource_dir, "lib", "darwin", "libclang_rt.ubsan_osx_dynamic.dylib"))
            elseif  target:is_plat("linux") then
                target:add("links", path.join(clang_resource_dir, "lib", "linux", "libclang_rt.ubsan_standalone-$(shell uname -m).a"))
                target:add("links", path.join(clang_resource_dir, "lib", "linux", "libclang_rt.ubsan_standalone_cxx-$(shell uname -m).a"))
            end
        end
    end)
rule_end()


rule("cpp_common")
    add_deps("cpp_sanitizer")
rule_end()

-- Platform-specific

if is_plat("linux") then
    set_toolchains("llvm")
    set_runtimes("c++_shared")
elseif is_plat("macosx") then
    set_toolchains("llvm")
end

-- Mode-specific

if is_mode("debug") then
    set_policy("build.sanitizer.leak", true)
    set_policy("build.sanitizer.undefined", true)
end

-- Targets

target("cubic-standalone")
    set_kind("binary")
    add_rules("cpp_common")

    add_files("src/bin/standalone.main.cpp")
