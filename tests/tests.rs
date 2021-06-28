use anyhow::Context;
use wat::parse_str as wat_to_wasm;
use wizer::Wizer;

fn run_wat(args: &[wasmtime::Val], expected: i32, wat: &str) -> anyhow::Result<()> {
    let _ = env_logger::try_init();
    let wasm = wat_to_wasm(wat)?;
    run_wasm(args, expected, &wasm)
}

fn run_wasm(args: &[wasmtime::Val], expected: i32, wasm: &[u8]) -> anyhow::Result<()> {
    let _ = env_logger::try_init();

    let mut wizer = Wizer::new();
    wizer.allow_wasi(true);
    wizer.wasm_multi_memory(true);
    wizer.wasm_module_linking(true);
    let wasm = wizer.run(&wasm)?;
    log::debug!(
        "=== Wizened Wasm ==========================================================\n\
         {}\n\
         ===========================================================================",
        wasmprinter::print_bytes(&wasm).unwrap()
    );

    let mut config = wasmtime::Config::new();
    config.cache_config_load_default().unwrap();
    config.wasm_multi_memory(true);
    config.wasm_multi_value(true);
    config.wasm_module_linking(true);

    let engine = wasmtime::Engine::new(&config)?;
    let store = wasmtime::Store::new(&engine);
    let module =
        wasmtime::Module::new(store.engine(), wasm).context("Wasm test case failed to compile")?;

    let dummy_module = wasmtime::Module::new(store.engine(), &wat::parse_str("(module)")?)?;
    let dummy_instance = wasmtime::Instance::new(&store, &dummy_module, &[])?;

    let mut linker = wasmtime::Linker::new(&store);
    linker
        .define_name("dummy_func", wasmtime::Func::wrap(&store, || {}))?
        .define("env", "f", wasmtime::Func::wrap(&store, || {}))?
        .define_name("dummy_instance", dummy_instance)?;
    let ctx = wasi_cap_std_sync::WasiCtxBuilder::new().build();
    let wasi = wasmtime_wasi::Wasi::new(&store, ctx);
    wasi.add_to_linker(&mut linker)?;
    let instance = linker.instantiate(&module)?;

    let run = instance
        .get_func("run")
        .ok_or_else(|| anyhow::anyhow!("the test Wasm module does not export a `run` function"))?;

    let actual = run.call(args)?;
    anyhow::ensure!(actual.len() == 1, "expected one result");
    let actual = match actual[0] {
        wasmtime::Val::I32(x) => x,
        _ => anyhow::bail!("expected an i32 result"),
    };
    anyhow::ensure!(
        expected == actual,
        "expected `{}`, found `{}`",
        expected,
        actual,
    );

    Ok(())
}

#[test]
fn basic_global() -> anyhow::Result<()> {
    run_wat(
        &[],
        42,
        r#"
(module
  (global $g (mut i32) i32.const 0)
  (func (export "wizer.initialize")
    i32.const 42
    global.set $g)
  (func (export "run") (result i32)
    global.get $g))
        "#,
    )
}

#[test]
fn basic_memory() -> anyhow::Result<()> {
    run_wat(
        &[],
        42,
        r#"
(module
  (memory 1)
  (func (export "wizer.initialize")
    i32.const 0
    i32.const 42
    i32.store offset=1337)
  (func (export "run") (result i32)
    i32.const 0
    i32.load offset=1337))
        "#,
    )
}

#[test]
fn multi_memory() -> anyhow::Result<()> {
    run_wat(
        &[],
        42,
        r#"
(module
  (memory $m1 1)
  (memory $m2 1)
  (func (export "wizer.initialize")
    i32.const 0
    i32.const 41
    i32.store (memory $m1) offset=1337
    i32.const 0
    i32.const 1
    i32.store (memory $m2) offset=1337)
  (func (export "run") (result i32)
    i32.const 0
    i32.load (memory $m1) offset=1337
    i32.const 0
    i32.load (memory $m2) offset=1337
    i32.add))
"#,
    )
}

#[test]
fn reject_imported_memory() -> anyhow::Result<()> {
    assert!(run_wat(
        &[],
        42,
        r#"
(module
  (import "" "" (memory 1)))
"#,
    )
    .is_err());
    Ok(())
}

#[test]
fn reject_imported_global() -> anyhow::Result<()> {
    assert!(run_wat(
        &[],
        42,
        r#"
(module
  (import "" "" (global i32)))
"#,
    )
    .is_err());
    Ok(())
}

#[test]
fn reject_imported_table() -> anyhow::Result<()> {
    assert!(run_wat(
        &[],
        42,
        r#"
(module
  (import "" "" (table)))
"#,
    )
    .is_err());
    Ok(())
}

#[test]
fn reject_bulk_memory() -> anyhow::Result<()> {
    let result = run_wat(
        &[],
        42,
        r#"
(module
  (table 3 funcref)

  (func $f (result i32) (i32.const 0))
  (func $g (result i32) (i32.const 0))
  (func $h (result i32) (i32.const 0))

  (func (export "main")
    i32.const 0
    i32.const 1
    i32.const 1
    table.copy)

  (elem (i32.const 0) $f $g $h)
)
"#,
    );
    assert!(result.is_err());

    let err = result.unwrap_err();
    assert!(err
        .to_string()
        .contains("unsupported `table.copy` instruction"));

    Ok(())
}

#[test]
fn accept_module_linking_import_memory() -> anyhow::Result<()> {
    run_wat(
        &[],
        42,
        r#"
(module
  (module $A
    (memory (export "memory") 1))
  (instance $a (instantiate $A))

  (module $B
    (import "x" (instance $x (export "memory" (memory 1)))))
  (instance $b (instantiate $B (import "x" (instance $a))))

  (func (export "wizer.initialize")
    nop)

  (func (export "run") (result i32)
    i32.const 42)
)
"#,
    )
}

#[test]
fn accept_module_linking_import_global() -> anyhow::Result<()> {
    run_wat(
        &[],
        42,
        r#"
(module
  (module $A
    (global (export "global") i32 (i32.const 1337)))
  (instance $a (instantiate $A))

  (module $B
    (import "x" (instance $x (export "global" (global i32)))))
  (instance $b (instantiate $B (import "x" (instance $a))))

  (func (export "wizer.initialize")
    nop)

  (func (export "run") (result i32)
    i32.const 42)
)
"#,
    )
}

#[test]
fn accept_module_linking_import_table() -> anyhow::Result<()> {
    run_wat(
        &[],
        42,
        r#"
(module
  (module $A
    (table (export "table") 0 funcref))
  (instance $a (instantiate $A))

  (module $B
    (import "x" (instance $x (export "table" (table 0 funcref)))))
  (instance $b (instantiate $B (import "x" (instance $a))))

  (func (export "wizer.initialize")
    nop)

  (func (export "run") (result i32)
    i32.const 42)
)
"#,
    )
}

#[test]
fn module_linking_actually_works() -> anyhow::Result<()> {
    run_wat(
        &[],
        42,
        r#"
(module
  (module $memory-module
    (memory (export "memory") 1))
  (instance $memory-instance (instantiate $memory-module))

  (module $use-memory
    (import "x" (instance $m (export "memory" (memory 1))))
    (func (export "init")
      i32.const 0
      i32.const 42
      i32.store (memory $m "memory") offset=1337))
  (instance $use-memory-instance
    (instantiate $use-memory
      (import "x" (instance $memory-instance))))

  (func (export "wizer.initialize")
    (call (func $use-memory-instance "init")))

  (func (export "run") (result i32)
    i32.const 0
    i32.load (memory $memory-instance "memory") offset=1337)
)
"#,
    )
}

#[test]
fn module_linking_nested_instantiations_1() -> anyhow::Result<()> {
    run_wat(
        &[],
        8,
        r#"
(module
  (module $A
    (import "global" (global (mut i32)))

    (module $B
      (import "global" (global (mut i32)))

        (module $C
          (import "global" (global (mut i32)))

          (func (export "f")
            i32.const 1
            global.get 0
            i32.add
            global.set 0
          )
        )

        (instance $c1 (instantiate $C (import "global" (global 0))))
        (instance $c2 (instantiate $C (import "global" (global 0))))

        (func (export "f")
          call (func $c1 "f")
          call (func $c2 "f")
       )
    )

    (instance $b1 (instantiate $B (import "global" (global 0))))
    (instance $b2 (instantiate $B (import "global" (global 0))))

    (func (export "f")
      call (func $b1 "f")
      call (func $b2 "f")
    )
  )

  (module $DefinesGlobal
    (global (export "global") (mut i32) (i32.const 0)))
  (instance $global_instance (instantiate $DefinesGlobal))

  (instance $a1 (instantiate $A (import "global" (global $global_instance "global"))))
  (instance $a2 (instantiate $A (import "global" (global $global_instance "global"))))

  (func (export "wizer.initialize")
    call (func $a1 "f")
    call (func $a2 "f"))

  (func (export "run") (result i32)
    global.get (global $global_instance "global"))
)
"#,
    )
}

#[test]
fn module_linking_nested_instantiations_0() -> anyhow::Result<()> {
    run_wat(
        &[],
        42,
        r#"
(module
  (module $A
    (import "global" (global (mut i32)))

    (module $B
      (import "global" (global (mut i32)))

       (func (export "f")
         i32.const 42
         global.set 0
       )
    )

    (instance $b (instantiate $B (import "global" (global 0))))

    (func (export "f")
      call (func $b "f")
    )
  )

  (module $G
    (global (export "global") (mut i32) (i32.const 0)))

  (instance $g (instantiate $G))

  (instance $a (instantiate $A (import "global" (global $g "global"))))

  (func (export "wizer.initialize")
    call (func $a "f")
  )

  (func (export "run") (result i32)
    global.get (global $g "global")
  )
)
"#,
    )
}

// Test that we handle repeated and interleaved initial sections.
#[test]
fn multiple_initial_sections() -> anyhow::Result<()> {
    run_wat(
        &[],
        42,
        r#"
(module
  ;; Module section.
  (module $A
    (memory (export "memory") 1)
  )

  ;; Instance section.
  (instance $a (instantiate $A))

  ;; Alias section.
  (alias $a "memory" (memory $memory))

  ;; Module section.
  (module $B
    (import "memory" (memory 1))
    (func (export "init")
      i32.const 0
      i32.const 42
      i32.store offset=1337
    )
  )

  ;; Instance section.
  (instance $b (instantiate $B (import "memory" (memory $memory))))

  ;; Alias section.
  (alias $b "init" (func $b-init))

  ;; Module section.
  (module $C
    (import "memory" (memory 1))
    (func (export "run") (result i32)
      i32.const 0
      i32.load offset=1337
    )
  )

  ;; Instance section.
  (instance $c (instantiate $C (import "memory" (memory $memory))))

  ;; Alias section.
  (alias $c "run" (func $c-run))

  ;; Done with initial sections.

  (func (export "wizer.initialize")
    call $b-init
  )

  (func (export "run") (result i32)
    call $c-run
  )
)
"#,
    )
}

#[test]
fn start_sections_in_nested_modules() -> anyhow::Result<()> {
    run_wat(
        &[],
        42,
        r#"
(module
  (module $A
    (import "global" (global $g (mut i32)))
    (func $init
      i32.const 41
      global.set $g)
    (start $init)
  )

  (module $B
    (global (export "global") (mut i32) (i32.const 0))
  )

  (instance $b (instantiate $B))
  (alias $b "global" (global $g))
  (instance $a (instantiate $A (import "global" (global $g))))

  (func (export "wizer.initialize")
    global.get $g
    i32.const 1
    i32.add
    global.set $g
  )
  (func (export "run") (result i32)
    global.get $g
  )
)
"#,
    )
}

#[test]
fn allow_function_imports_module_linking() -> anyhow::Result<()> {
    // Make sure that the umbrella module passes imports through to its
    // instantiation of the root, and that the root can pass them along to its
    // nested instantiations as well.
    run_wat(
        &[],
        42,
        r#"
(module
  (import "dummy_func" (func $dummy))
  (module $A
    (import "dummy_func" (func)))
  (instance (instantiate $A (import "dummy_func" (func $dummy))))
  (func (export "wizer.initialize")
    nop
  )
  (func (export "run") (result i32)
    i32.const 42
  )
)
"#,
    )
}

#[test]
fn outer_module_alias() -> anyhow::Result<()> {
    run_wat(
        &[],
        42,
        r#"
(module
  (module $A
    (global (export "g") (mut i32) (i32.const 0))
  )

  (module $B
    (alias outer 0 0 (module $A))
    (instance $a (instantiate $A))
    (func (export "init")
      i32.const 42
      global.set (global $a "g")
    )
    (func (export "run") (result i32)
      global.get (global $a "g")
    )
  )
  (instance $b (instantiate $B))

  (func (export "wizer.initialize")
    call (func $b "init")
  )
  (func (export "run") (result i32)
    call (func $b "run")
  )
)
"#,
    )
}

#[test]
fn instance_alias_without_entry_in_type_section() -> anyhow::Result<()> {
    run_wat(
        &[],
        42,
        r#"
(module
  (module $CHILD
    (module $a)
    (instance $a (instantiate $a))
    (export "a" (instance $a)))
  (instance $child (instantiate $CHILD))

  ;; This root module doesn't ever declare an instance type for this alias.
  (alias $child "a" (instance $no_type_for_this_instance))

  (func (export "wizer.initialize")
    nop
  )
  (func (export "run") (result i32)
    i32.const 42
  )
)
"#,
    )
}

#[test]
fn two_level_imports_and_implicit_instance_imports() -> anyhow::Result<()> {
    run_wat(
        &[],
        42,
        r#"
(module
  ;; First, import an instance to make sure that we are accounting for already
  ;; imported instances when forwarding implicit instances and are getting the
  ;; index space correct.
  (import "dummy_instance" (instance))

  ;; This implicitly creates an instance import like:
  ;;
  ;;     (import (env (instance (export "f" (func $f)))))
  ;;
  ;; We will have to forward this implicit instance from the umbrella to the
  ;; root instantiation.
  (import "env" "f" (func $f))

  (module $A
    (import "env" "f" (func)))

  ;; Pass that implicit instance through when instantiating `$A`.
  (instance $a (instantiate $A (import "env" (instance 1))))

  (func (export "wizer.initialize")
    nop
  )
  (func (export "run") (result i32)
    i32.const 42
  )
)
"#,
    )
}

#[test]
fn implicit_instance_imports_and_other_instances() -> anyhow::Result<()> {
    // Test how implicit instance import injection interacts with explicit
    // instance imports and explicit instantiations.
    run_wat(
        &[],
        42,
        r#"
(module
  (module $A
    ;; This implicitly creates an instance import like:
    ;;
    ;;     (import (env (instance (export "f" (func $f (result i32))))))
    (import "env" "f" (func $f (result i32)))

    (import "env2" (instance $env2 (export "g" (func (result i32)))))

    (module $B
      (func (export "h") (result i32)
        i32.const 1
      )
    )
    (instance $b (instantiate $B))

    (func (export "run") (result i32)
      call $f
      call (func $env2 "g")
      call (func $b "h")
      i32.add
      i32.add
    )
  )

  (module $Env
    (func (export "f") (result i32)
      i32.const 2
    )
  )
  (instance $env (instantiate $Env))

  (module $Env2
    (func (export "g") (result i32)
      i32.const 39
    )
  )
  (instance $env2 (instantiate $Env2))

  (instance $a (instantiate $A
                 (import "env" (instance $env))
                 (import "env2" (instance $env2))))

  (func (export "wizer.initialize")
    nop
  )
  (func (export "run") (result i32)
    call (func $a "run")
  )
)
"#,
    )
}

#[test]
fn rust_regex() -> anyhow::Result<()> {
    run_wasm(
        &[wasmtime::Val::I32(13)],
        42,
        &include_bytes!("./regex_test.wasm")[..],
    )
}

#[test]
fn data_segment_at_end_of_memory() -> anyhow::Result<()> {
    // Test that we properly synthesize data segments for data at the end of
    // memory.
    run_wat(
        &[],
        42,
        r#"
(module
  (memory 1)
  (func (export "wizer.initialize")
    ;; Store 42 to the last byte in memory.
    i32.const 0
    i32.const 42
    i32.store8 offset=65535
  )
  (func (export "run") (result i32)
    i32.const 0
    i32.load8_u offset=65535
  )
)
"#,
    )
}

#[test]
fn rename_functions() -> anyhow::Result<()> {
    let wat = r#"
(module
 (func (export "wizer.initialize"))
 (func (export "func_a") (result i32)
   i32.const 1)
 (func (export "func_b") (result i32)
   i32.const 2)
 (func (export "func_c") (result i32)
   i32.const 3))
  "#;

    let wasm = wat_to_wasm(wat)?;
    let mut wizer = Wizer::new();
    wizer.allow_wasi(true);
    wizer.func_rename("func_a", "func_b");
    wizer.func_rename("func_b", "func_c");
    let wasm = wizer.run(&wasm)?;
    let wat = wasmprinter::print_bytes(&wasm)?;

    let expected_wat = r#"
(module
  (type (;0;) (func))
  (type (;1;) (func (result i32)))
  (func (;0;) (type 0))
  (func (;1;) (type 1) (result i32)
    i32.const 1)
  (func (;2;) (type 1) (result i32)
    i32.const 2)
  (func (;3;) (type 1) (result i32)
    i32.const 3)
  (export "func_a" (func 2))
  (export "func_b" (func 3)))
  "#;

    assert_eq!(wat.trim(), expected_wat.trim());
    Ok(())
}

#[test]
fn renames_and_module_linking() -> anyhow::Result<()> {
    let wat = r#"
(module
  (module $A
    (func (export "a") (result i32)
      i32.const 1)
    (func (export "b") (result i32)
      i32.const 2)
    (func (export "c") (result i32)
      i32.const 3)
  )
  (instance $a (instantiate $A))
  (func (export "wizer.initialize")
    nop
  )
  (func (export "a") (result i32)
    call (func $a "a")
  )
  (func (export "b") (result i32)
    call (func $a "b")
  )
  (func (export "c") (result i32)
    call (func $a "c")
  )
)
  "#;

    let wasm = wat_to_wasm(wat)?;
    let mut wizer = Wizer::new();
    wizer.wasm_module_linking(true);
    wizer.allow_wasi(true);
    wizer.func_rename("a", "b");
    wizer.func_rename("b", "c");
    let wasm = wizer.run(&wasm)?;
    let wat = wasmprinter::print_bytes(&wasm)?;

    let expected_wat = r#"
  (alias 1 "b" (func (;0;)))
  (alias 1 "c" (func (;1;)))
  (export "a" (func 0))
  (export "b" (func 1)))
  "#;

    assert!(wat.trim().ends_with(expected_wat.trim()));
    Ok(())
}
