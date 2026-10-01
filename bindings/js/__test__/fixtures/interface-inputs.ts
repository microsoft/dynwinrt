// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { FileIO } from './generated/windows/storage/FileIO.js'
import { StorageFile } from './generated/windows/storage/StorageFile.js'
import { StorageFolder } from './generated/windows/storage/StorageFolder.js'
import { IStorageFile } from './generated/windows/storage/IStorageFile.js'
import { IStorageFolder } from './generated/windows/storage/IStorageFolder.js'
import { IStorageItem } from './generated/windows/storage/IStorageItem.js'
import { NameCollisionOption } from './generated/windows/storage/NameCollisionOption.js'
import { Buffer as WinRTBuffer } from './generated/windows/storage/streams/Buffer.js'
import { IBuffer } from './generated/windows/storage/streams/IBuffer.js'
import { Uri } from './generated/windows/foundation/Uri.js'
import { IStringable } from './generated/windows/foundation/IStringable.js'
import { FileIO as PackageFileIO, StorageFile as PackageStorageFile, releaseProjected } from 'projected-storage'

export async function roundtrip(path: string, text: string): Promise<string> {
  const file = await StorageFile.getFileFromPathAsync(path)
  try {
    await FileIO.writeTextAsync(file, text)
    return await FileIO.readTextAsync(file)
  } finally {
    releaseProjected(file)
  }
}

export async function packageRoundtrip(path: string, text: string): Promise<string> {
  const file = await PackageStorageFile.getFileFromPathAsync(path)
  try {
    await PackageFileIO.writeTextAsync(file, text)
    return await PackageFileIO.readTextAsync(file)
  } finally {
    releaseProjected(file)
  }
}

// Compile-only controls; none of the negative calls execute.
export function inputContracts(file: StorageFile, folder: StorageFolder, uri: Uri, buffer: WinRTBuffer) {
  const fileInput: IStorageFile = file
  const folderInput: IStorageFolder = folder
  const fileItem: IStorageItem = file
  const folderItem: IStorageItem = folder
  const bufferInput: IBuffer = buffer
  const stringable: IStringable = uri
  const signal = new AbortController().signal
  FileIO.writeTextAsync(file, 'natural input')
  FileIO.readTextAsync(file)
  FileIO.writeBufferAsync(file, bufferInput)
  file.copyAsync(folder)
  file.copyAsync(folder, signal)
  file.copyAsync(folder, 'copy.txt')
  file.copyAsync(folder, 'copy.txt', NameCollisionOption.FailIfExists)
  file.moveAsync(folder)
  file.moveAsync(folder, 'move.txt')
  file.moveAsync(folder, 'move.txt', NameCollisionOption.FailIfExists)
  file.copyAndReplaceAsync(file)
  file.moveAndReplaceAsync(file)
  fileInput.copyOverloadDefaultNameAndOptions(folder)
  fileInput.copyOverloadDefaultOptions(folder, 'copy.txt')
  fileInput.copyOverload(folder, 'copy.txt', NameCollisionOption.FailIfExists)
  fileInput.moveOverloadDefaultNameAndOptions(folder)
  fileInput.moveOverloadDefaultOptions(folder, 'move.txt')
  fileInput.moveOverload(folder, 'move.txt', NameCollisionOption.FailIfExists)
  file.copyOverloadDefaultOptions(folderInput, 'class-alias.txt')
  file.copyOverloadDefaultOptions(folderInput, 'class-alias.txt', signal)
  file.moveOverloadDefaultOptions(folderInput, 'class-alias.txt')
  FileIO.readTextAsync(file.as(IStorageFile))
  FileIO.writeTextAsync(file.as(IStorageFile), 'explicit interface')
  file.as(IStorageFile).copyOverloadDefaultOptions(folder.as(IStorageFolder), 'legacy.txt')
  folder.createFileAsync('created.txt')
  folder.createFileAsyncOverloadDefaultOptions('alias.txt')
  folder.createFolderAsyncOverloadDefaultOptions('child')
  folder.getFilesAsyncOverloadDefaultOptionsStartAndCount()
  folder.getFoldersAsyncOverloadDefaultOptionsStartAndCount()
  folder.getItemsAsyncOverloadDefaultStartAndCount()
  fileItem.isOfType(0)
  file.isEqual(folderItem)
  stringable.toString()
  // @ts-expect-error Uri does not implement IStorageFile.
  FileIO.writeTextAsync(uri, 'wrong IID')
  // @ts-expect-error Uri does not implement IStorageFile.
  FileIO.readTextAsync(uri)
  // @ts-expect-error Uri does not implement IStorageFolder.
  file.copyAsync(uri)
  // @ts-expect-error Uri does not implement IStorageFolder.
  fileInput.moveOverloadDefaultNameAndOptions(uri)
  // @ts-expect-error A folder is not a file.
  FileIO.readTextAsync(folder)
  // @ts-expect-error A file is not a folder.
  file.copyAsync(file)
}
