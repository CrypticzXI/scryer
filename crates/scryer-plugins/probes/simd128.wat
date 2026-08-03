(module
  (func (export "probe") (result i32)
    (i32x4.extract_lane 0
      (v128.const i32x4 0 0 0 0))))
