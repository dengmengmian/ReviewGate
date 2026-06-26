# COBOL Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [COB1] A computation or MOVE result exceeds the PIC-defined digits, causing high-order truncation or overflow without ON SIZE ERROR handling.
- [COB2] Mixing numeric and alphanumeric fields in MOVE/comparison, causing sign, shift, or padding errors.
- [COB3] A WORKING-STORAGE field used in computation or accumulation without being initialized via VALUE.
- [COB4] An OCCURS table subscript outside the defined range, or missing bounds checks causing out-of-bounds access.
- [COB5] COMPUTE/DIVIDE without ROUNDED or a rounding mode, so fixed-point decimals are silently truncated, introducing monetary errors.
