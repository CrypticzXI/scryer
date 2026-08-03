(module
  (func (export "probe") (result i32)
    (i8x16.extract_lane_u 0
      (i8x16.relaxed_swizzle
        (v128.const i8x16 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15)
        (v128.const i8x16 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0)))))
