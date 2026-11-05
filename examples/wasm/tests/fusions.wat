(module
  ;; Translator fusions that rewrite or reorder ops (compile.lm), each
  ;; on the case that once broke it or could; $g covers a global's
  ;; update (globals in one array were tried, FRICTION.md). Prints with
  ;; env.print_i32 / print_f64.
  (import "env" "print_i32" (func $pi (param i32)))
  (import "env" "print_f64" (func $pf (param f64)))
  (memory 1)
  (data (i32.const 0) "\0a\00\00\00\14\00\00\00\1e\00\00\00\28\00\00\00")
  (global $g (mut i32) (i32.const 7))

  ;; `p + 4` tee'd into p as the store's address, then loaded from p
  ;; and tee'd into p again while that address is pending (I32_LOAD_TK,
  ;; and the flush of a pending local, which must not move before it).
  (func $tee (param i32) (result i32)
    (i32.store
      (local.tee 0 (i32.add (local.get 0) (i32.const 4)))
      (i32.add (local.tee 0 (i32.load (local.get 0))) (i32.const 1)))
    (i32.add (local.get 0) (i32.load (i32.const 4))))

  ;; Two moves where the second reads what the first wrote (b = a; c = b;
  ;; a merged MOV2 was tried, FRICTION.md).
  (func $movs (param i32 i32 i32) (result i32)
    (local.set 1 (local.get 0))
    (local.set 2 (local.get 1))
    (i32.add (i32.mul (local.get 2) (i32.const 100)) (local.get 1)))

  ;; select(a, b, a < b) and the swapped compare; f64 with a NaN.
  (func $min (param i32 i32) (result i32)
    (select (local.get 0) (local.get 1) (i32.lt_s (local.get 0) (local.get 1))))
  (func $max_swapped (param i32 i32) (result i32)
    (select (local.get 0) (local.get 1) (i32.lt_u (local.get 1) (local.get 0))))
  (func $fmin (param f64 f64) (result f64)
    (select (local.get 0) (local.get 1) (f64.lt (local.get 0) (local.get 1))))

  ;; A constant in a loop for an op without a K form (a constant slot).
  (func $rem (param i32) (result i32) (local i32)
    (loop
      (local.set 1 (i32.add (local.get 1) (i32.rem_u (local.get 0) (i32.const 7))))
      (local.set 0 (i32.sub (local.get 0) (i32.const 1)))
      (br_if 0 (local.get 0)))
    (local.get 1))

  ;; Loads from the sum of two locals, as a temporary (I32_LOAD_R) and
  ;; tee'd into the first one (I32_LOAD_TR, which also writes it); the
  ;; sum wraps at 32 bits (-4 + 8 is address 4, not 2^32 + 4).
  (func $rload (param i32 i32) (result i32)
    (i32.load (i32.add (local.get 0) (local.get 1))))
  (func $rtee (param i32 i32) (result i32)
    (i32.add
      (i32.load8_u (local.tee 0 (i32.add (local.get 0) (local.get 1))))
      (i32.mul (local.get 0) (i32.const 1000))))

  ;; Offsets near the end of the 16-bit field, which holds offset + size:
  ;; 65530 + 4 fits, 65532 + 4 does not (ADDR and I32_LOAD_X).
  (func $far (result i32)
    (i32.store offset=65530 (i32.const 0) (i32.const 11))
    (i32.store offset=65532 (i32.const 0) (i32.const 22))
    (i32.add (i32.load16_u offset=65530 (i32.const 0)) (i32.load offset=65532 (i32.const 0))))

  (func (export "main") (result i32)
    (call $pi (call $tee (i32.const 0)))
    (call $pi (i32.load (i32.const 0)))
    (call $pi (i32.load (i32.const 4)))
    (call $pi (call $movs (i32.const 3) (i32.const 0) (i32.const 0)))
    (call $pi (call $min (i32.const -5) (i32.const 2)))
    (call $pi (call $min (i32.const 9) (i32.const 2)))
    (call $pi (call $max_swapped (i32.const -1) (i32.const 2)))
    (call $pf (call $fmin (f64.const 1.5) (f64.const 2.5)))
    (call $pf (call $fmin (f64.const nan) (f64.const 2.5)))
    (call $pf (call $fmin (f64.const 2.5) (f64.const nan)))
    (call $pi (call $rem (i32.const 20)))
    (call $pi (call $rload (i32.const -4) (i32.const 8)))
    (call $pi (call $rload (i32.const 8) (i32.const 4)))
    (call $pi (call $rtee (i32.const 2) (i32.const 2)))
    (call $pi (call $far))
    (global.set $g (i32.add (global.get $g) (i32.const 5)))
    (call $pi (global.get $g))
    (i32.const 0)))
