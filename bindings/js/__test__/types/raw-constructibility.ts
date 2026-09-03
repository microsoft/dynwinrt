import {
  DynComRaw,
  DynComRawCleanup,
  DynComRawMemory,
  DynComRawOwnedComPointer,
  DynComRawPointer,
  DynComRawStructLayout,
  DynComRawUnionLayout,
  WinGuid,
} from '../../dist/com-unsafe-raw.js'

const pointerSize = DynComRaw.pointerSize()
const memory = DynComRawMemory.allocate(pointerSize, pointerSize)
const external = DynComRawPointer.fromAddress(0x1234n)
const nullPointer = DynComRawPointer.null()
memory.writePointer(0, external)
const owned = memory.pointer()
const callValue = owned.toValue()
const externalView = DynComRawMemory.fromUnsafePointer(owned, pointerSize, pointerSize)
const addressView = DynComRawMemory.fromUnsafeAddress(owned.address, pointerSize, pointerSize)
const structLayout = DynComRawStructLayout.fromDescriptor('{}')
const structValue = structLayout.createValue()
const structBytes = structLayout.readValueBytes(structValue)
const structType = structLayout.byValueType()
const structPointerType = structLayout.pointerType()
const unionLayout = DynComRawUnionLayout.fromDescriptor('{}')
const unionValue = unionLayout.createValue('field')
const unionBytes = unionLayout.readValueBytes(unionValue)
const assertedUnionBytes = unionLayout.assertActiveField(unionValue, 'field')
const unionPointerType = unionLayout.pointerType()
const unionByValueType = unionLayout.byValueType()
const borrowedManagedPointer = DynComRawPointer.fromManagedBorrowed(callValue)
const ownedManagedPointer = DynComRawOwnedComPointer.addRef(callValue)
const queriedManagedPointer = DynComRawOwnedComPointer.queryInterface(
  callValue,
  WinGuid.parse('00000000-0000-0000-c000-000000000046'),
)
const retainedManagedPointer = queriedManagedPointer.retain()
const ownedPointerView = ownedManagedPointer.pointer()
const detachedPointer = ownedManagedPointer.detach()
const adoptedPointer = DynComRawOwnedComPointer.adoptTransferred(detachedPointer)
const managedAgain = adoptedPointer.intoManaged()
const transferOwner = DynComRawOwnedComPointer.addRef(callValue)
transferOwner.transferTo(memory)
const assumedPointer = DynComRawOwnedComPointer.assumeTransferred(memory.readPointer(0))
DynComRawCleanup.coTaskMemFree(external)
void [
  pointerSize,
  memory,
  external,
  nullPointer,
  owned,
  callValue,
  externalView,
  addressView,
  structLayout,
  structBytes,
  structType,
  structPointerType,
  unionLayout,
  unionBytes,
  assertedUnionBytes,
  unionPointerType,
  unionByValueType,
  borrowedManagedPointer,
  ownedManagedPointer,
  queriedManagedPointer,
  retainedManagedPointer,
  ownedPointerView,
  detachedPointer,
  adoptedPointer,
  managedAgain,
  transferOwner,
  assumedPointer,
]

// @ts-expect-error Raw utility classes have no public runtime constructor.
new DynComRaw()
// @ts-expect-error Raw cleanup is static-only.
new DynComRawCleanup()
// @ts-expect-error Raw memory must be created with DynComRawMemory.allocate().
new DynComRawMemory()
// @ts-expect-error Raw pointers must use fromAddress(), null(), or memory.pointer().
new DynComRawPointer()
// @ts-expect-error Raw COM owners must use an ownership factory.
new DynComRawOwnedComPointer()
// @ts-expect-error Raw struct layouts must use fromDescriptor().
new DynComRawStructLayout()
// @ts-expect-error Raw union layouts must use fromDescriptor().
new DynComRawUnionLayout()

// @ts-expect-error A private constructor prevents subclassing.
class InvalidRaw extends DynComRaw {}
// @ts-expect-error A private constructor prevents subclassing.
class InvalidRawCleanup extends DynComRawCleanup {}
// @ts-expect-error A private constructor prevents subclassing.
class InvalidRawMemory extends DynComRawMemory {}
// @ts-expect-error A private constructor prevents subclassing.
class InvalidRawPointer extends DynComRawPointer {}
// @ts-expect-error A private constructor prevents subclassing.
class InvalidRawOwnedComPointer extends DynComRawOwnedComPointer {}
// @ts-expect-error A private constructor prevents subclassing.
class InvalidRawStructLayout extends DynComRawStructLayout {}
// @ts-expect-error A private constructor prevents subclassing.
class InvalidRawUnionLayout extends DynComRawUnionLayout {}
void [
  InvalidRaw,
  InvalidRawCleanup,
  InvalidRawMemory,
  InvalidRawPointer,
  InvalidRawOwnedComPointer,
  InvalidRawStructLayout,
  InvalidRawUnionLayout,
]
