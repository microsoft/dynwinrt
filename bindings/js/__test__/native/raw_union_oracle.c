#include <stddef.h>
#include <stdint.h>
#include <string.h>

#define ABI __stdcall
#define STATIC_ASSERT(name, expression) typedef char static_assert_##name[(expression) ? 1 : -1]

typedef union { uint8_t value; } RU1;
typedef union { uint8_t bytes[2]; uint16_t value; } RU2;
typedef union { uint8_t bytes[3]; } RU3;
typedef union { uint8_t bytes[4]; uint32_t value; } RU4;
typedef union { uint8_t bytes[5]; } RU5;
typedef union { uint8_t bytes[6]; } RU6;
typedef union { uint8_t bytes[7]; } RU7;
typedef union { uint8_t bytes[8]; uint64_t value; } RU8;
typedef union { uint64_t values[2]; } RU16;
typedef union { uint64_t values[3]; } RU24;
typedef union { float scalar; float values[1]; } RHFA1;
typedef union { float scalar; float values[2]; } RHFA2;
typedef union { float scalar; float values[4]; } RHFA4;
typedef union { float values[2]; uint64_t integer; } RMIXED;
typedef union { double scalar; double values[3]; } RDHFA3;
typedef union { RHFA2 inner; float values[2]; } RNESTED_UNION;
typedef struct { RHFA2 inner; float tail; } RNESTED_STRUCT;
typedef struct { RMIXED inner; float tail; uint32_t tag; } RMIXED_NESTED_STRUCT;

STATIC_ASSERT(u1_size, sizeof(RU1) == 1);
STATIC_ASSERT(u2_size, sizeof(RU2) == 2);
STATIC_ASSERT(u3_size, sizeof(RU3) == 3);
STATIC_ASSERT(u4_size, sizeof(RU4) == 4);
STATIC_ASSERT(u5_size, sizeof(RU5) == 5);
STATIC_ASSERT(u6_size, sizeof(RU6) == 6);
STATIC_ASSERT(u7_size, sizeof(RU7) == 7);
STATIC_ASSERT(u8_size, sizeof(RU8) == 8);
STATIC_ASSERT(u16_size, sizeof(RU16) == 16);
STATIC_ASSERT(u24_size, sizeof(RU24) == 24);
STATIC_ASSERT(hfa1_size, sizeof(RHFA1) == 4);
STATIC_ASSERT(hfa2_size, sizeof(RHFA2) == 8);
STATIC_ASSERT(hfa4_size, sizeof(RHFA4) == 16);
STATIC_ASSERT(mixed_size, sizeof(RMIXED) == 8);
STATIC_ASSERT(dhfa3_size, sizeof(RDHFA3) == 24);
STATIC_ASSERT(nested_union_size, sizeof(RNESTED_UNION) == 8);
STATIC_ASSERT(nested_struct_size, sizeof(RNESTED_STRUCT) == 12);
STATIC_ASSERT(mixed_nested_struct_size, sizeof(RMIXED_NESTED_STRUCT) == 16);
STATIC_ASSERT(u1_alignment, __alignof(RU1) == 1);
STATIC_ASSERT(u2_alignment, __alignof(RU2) == 2);
STATIC_ASSERT(u3_alignment, __alignof(RU3) == 1);
STATIC_ASSERT(u4_alignment, __alignof(RU4) == 4);
STATIC_ASSERT(u5_alignment, __alignof(RU5) == 1);
STATIC_ASSERT(u6_alignment, __alignof(RU6) == 1);
STATIC_ASSERT(u7_alignment, __alignof(RU7) == 1);
STATIC_ASSERT(nested_struct_tail, offsetof(RNESTED_STRUCT, tail) == 8);
STATIC_ASSERT(u8_alignment, __alignof(RU8) == 8);
STATIC_ASSERT(u16_alignment, __alignof(RU16) == 8);
STATIC_ASSERT(u24_alignment, __alignof(RU24) == 8);
STATIC_ASSERT(hfa1_alignment, __alignof(RHFA1) == 4);
STATIC_ASSERT(hfa2_alignment, __alignof(RHFA2) == 4);
STATIC_ASSERT(hfa4_alignment, __alignof(RHFA4) == 4);
STATIC_ASSERT(mixed_alignment, __alignof(RMIXED) == 8);
STATIC_ASSERT(dhfa3_alignment, __alignof(RDHFA3) == 8);
STATIC_ASSERT(nested_union_alignment, __alignof(RNESTED_UNION) == 4);
STATIC_ASSERT(nested_struct_alignment, __alignof(RNESTED_STRUCT) == 4);
STATIC_ASSERT(mixed_nested_struct_alignment, __alignof(RMIXED_NESTED_STRUCT) == 8);
STATIC_ASSERT(mixed_nested_struct_tail, offsetof(RMIXED_NESTED_STRUCT, tail) == 8);
STATIC_ASSERT(mixed_nested_struct_tag, offsetof(RMIXED_NESTED_STRUCT, tag) == 12);
STATIC_ASSERT(union_fields_begin_at_zero, offsetof(RU8, value) == 0 && offsetof(RHFA2, values) == 0);

RU1 ABI raw_union_c_return_u1(void* self) { RU1 value = { 0x7a }; (void)self; return value; }
RU2 ABI raw_union_c_return_u2(void* self) { RU2 value; value.value = 0x7a6b; (void)self; return value; }
RU3 ABI raw_union_c_return_u3(void* self) { RU3 value = { { 1, 2, 3 } }; (void)self; return value; }
RU4 ABI raw_union_c_return_u4(void* self) { RU4 value; value.value = 0x7a6b5c4d; (void)self; return value; }
RU5 ABI raw_union_c_return_u5(void* self) { RU5 value = { { 1, 2, 3, 4, 5 } }; (void)self; return value; }
RU6 ABI raw_union_c_return_u6(void* self) { RU6 value = { { 1, 2, 3, 4, 5, 6 } }; (void)self; return value; }
RU7 ABI raw_union_c_return_u7(void* self) { RU7 value = { { 1, 2, 3, 4, 5, 6, 7 } }; (void)self; return value; }
RU8 ABI raw_union_c_return_u8(void* self) { RU8 value; value.value = UINT64_C(0x7a6b5c4d3e2f1a0b); (void)self; return value; }
RU16 ABI raw_union_c_return_u16(void* self) { RU16 value = { { UINT64_C(0x1111222233334444), UINT64_C(0x5555666677778888) } }; (void)self; return value; }
RU24 ABI raw_union_c_return_u24(void* self) { RU24 value = { { 1, 2, 3 } }; (void)self; return value; }
RHFA1 ABI raw_union_c_return_hfa1(void* self) { RHFA1 value; value.scalar = 1.25f; (void)self; return value; }
RHFA2 ABI raw_union_c_return_hfa2(void* self) { RHFA2 value; value.values[0] = 1.25f; value.values[1] = 2.5f; (void)self; return value; }
RHFA4 ABI raw_union_c_return_hfa4(void* self) { RHFA4 value; value.values[0] = 1.0f; value.values[1] = 2.0f; value.values[2] = 3.0f; value.values[3] = 4.0f; (void)self; return value; }
RMIXED ABI raw_union_c_return_mixed(void* self) { RMIXED value; value.integer = UINT64_C(0x1020304050607080); (void)self; return value; }
RDHFA3 ABI raw_union_c_return_dhfa3(void* self) { RDHFA3 value; value.values[0] = 1.0; value.values[1] = 2.0; value.values[2] = 3.0; (void)self; return value; }
RNESTED_UNION ABI raw_union_c_return_nested_union(void* self) { RNESTED_UNION value; value.inner.values[0] = 5.0f; value.inner.values[1] = 6.0f; (void)self; return value; }
RNESTED_STRUCT ABI raw_union_c_return_nested_struct(void* self) { RNESTED_STRUCT value; value.inner.values[0] = 1.0f; value.inner.values[1] = 2.0f; value.tail = 3.0f; (void)self; return value; }
RMIXED_NESTED_STRUCT ABI raw_union_c_return_mixed_nested_struct(void* self) {
  RMIXED_NESTED_STRUCT value;
  value.inner.integer = UINT64_C(0x1020304050607080);
  value.tail = 3.5f;
  value.tag = UINT32_C(0xa1b2c3d4);
  (void)self;
  return value;
}

int32_t ABI raw_union_c_u8_first(void* self, RU8 value, uint32_t a, uint32_t b, uint32_t c) {
  return self && value.value == UINT64_C(0x1122334455667788) && a == 11 && b == 22 && c == 33 ? 0 : -1;
}

int32_t ABI raw_union_c_u8_fourth(void* self, uint32_t a, uint32_t b, uint32_t c, RU8 value, uint32_t tail) {
  return self && value.value == UINT64_C(0x1122334455667788) && a == 11 && b == 22 && c == 33 && tail == 44 ? 0 : -1;
}

int32_t ABI raw_union_c_u16_post_register(void* self, uint32_t a, uint32_t b, uint32_t c, uint32_t d, RU16 value) {
  return self && a == 1 && b == 2 && c == 3 && d == 4
      && value.values[0] == UINT64_C(0x1111222233334444)
      && value.values[1] == UINT64_C(0x5555666677778888) ? 0 : -1;
}

int32_t ABI raw_union_c_u16_guarded_copy(void* self, RU16 value, uint8_t* destination) {
  size_t index;
  if (!self || !destination || destination[-1] != 0xa5 || destination[sizeof(value)] != 0x5a) {
    return -1;
  }
  for (index = 0; index < sizeof(value); ++index) {
    destination[index] = ((const uint8_t*)&value)[index];
  }
  return destination[-1] == 0xa5 && destination[sizeof(value)] == 0x5a ? 0 : -1;
}

uint64_t ABI raw_union_c_u16_mutate_local(
    void* self,
    uint32_t before,
    RU16 value,
    const uint8_t* original,
    uint32_t after) {
  RU16 expected = { { UINT64_C(0x1111222233334444), UINT64_C(0x5555666677778888) } };
  if (!self || before != UINT32_C(0x11223344) || after != UINT32_C(0x55667788)
      || !original || original[-1] != 0xa5 || original[sizeof(value)] != 0x5a
      || memcmp(original, &expected, sizeof(value)) != 0
      || memcmp(&value, &expected, sizeof(value)) != 0) {
    return 0;
  }
  value.values[0] ^= UINT64_C(0xffff0000ffff0000);
  value.values[1] += UINT64_C(0x0102030405060708);
  if (original[-1] != 0xa5 || original[sizeof(value)] != 0x5a
      || memcmp(original, &expected, sizeof(value)) != 0) {
    return 0;
  }
  return value.values[0] ^ value.values[1];
}

int32_t ABI raw_union_c_mixed_nested_struct_input(
    void* self,
    uint32_t before,
    RMIXED_NESTED_STRUCT value,
    uint32_t after) {
  return self
      && before == UINT32_C(0x13579bdf)
      && value.inner.integer == UINT64_C(0x1020304050607080)
      && value.tail == 3.5f
      && value.tag == UINT32_C(0xa1b2c3d4)
      && after == UINT32_C(0x2468ace0) ? 0 : -1;
}

int32_t ABI raw_union_c_hfa1_input(void* self, RHFA1 value, uint32_t canary) {
  return self && value.scalar == 1.25f && canary == 0xa1a2a3a4 ? 0 : -1;
}

int32_t ABI raw_union_c_hfa2_input(void* self, RHFA2 value, uint32_t canary) {
  return self && value.values[0] == 1.25f && value.values[1] == 2.5f && canary == 0xb1b2b3b4 ? 0 : -1;
}

int32_t ABI raw_union_c_hfa4_input(void* self, RHFA4 value, uint32_t canary) {
  return self && value.values[0] == 1.0f && value.values[3] == 4.0f && canary == 0xc1c2c3c4 ? 0 : -1;
}

int32_t ABI raw_union_c_nested_union_input(void* self, RNESTED_UNION value, uint32_t canary) {
  return self && value.inner.values[0] == 5.0f && value.inner.values[1] == 6.0f && canary == 0xd1d2d3d4 ? 0 : -1;
}
