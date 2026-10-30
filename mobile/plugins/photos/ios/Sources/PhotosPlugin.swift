import Foundation
import ImageIO
import Photos
import PhotosUI
import SwiftRs
import Tauri
import UIKit
import UniformTypeIdentifiers

struct PhotosArgs: Decodable {
  /// An album's local identifier, or nil for every photo.
  let album: String?
}

struct OriginalArgs: Decodable {
  let id: Int64
  /// Longest side to edit at: anything larger is decoded down to it.
  let maxEdge: Int
  /// Where to write the file.
  let path: String
}

struct FileArgs: Decodable {
  let id: Int64
  /// Where to write it.
  let path: String
}

struct MetadataArgs: Decodable {
  /// The photo whose details to copy.
  let id: Int64
  /// The JPEG to copy them into.
  let path: String
  /// When it was taken, for a photo whose file doesn't say.
  let takenMs: Int64
}

struct ImageArgs: Decodable {
  let id: Int64
  /// Longest side, in pixels.
  let size: Int
  /// Where to write the JPEG.
  let path: String
}

struct AccessReply: Encodable { let access: String }
struct Album: Encodable {
  let id: String
  let name: String
  let count: Int
}
struct AlbumsReply: Encodable { let albums: [Album] }
struct PhotosReply: Encodable { let photos: [[Int64]] }
struct OriginalReply: Encodable {
  let name: String
  let takenMs: Int64
  let copied: Bool
}
struct FileReply: Encodable {
  let name: String
  let takenMs: Int64
  let mime: String
}
struct Done: Encodable {}

struct PhotosError: LocalizedError {
  let message: String
  init(_ message: String) { self.message = message }
  var errorDescription: String? { message }
}

/// The phone's own photos for On this phone (docs/MOBILE.md section 4), from
/// PhotoKit: whether the app may see them, the albums, the photos newest
/// first, and JPEGs of any of them at a given size. iOS's own decoders do the
/// work, so HEIC and every format the phone reads come for free.
///
/// The app keys a phone photo by number (its cache files, its edits), as
/// Android's MediaStore does; PhotoKit names one by a string. Each is
/// numbered by a hash of that string, and the numbers seen are mapped back.
class PhotosPlugin: Plugin, PHPhotoLibraryChangeObserver {
  /// Off the main thread; a few at once, as a grid asks for many thumbnails.
  private let worker: OperationQueue = {
    let queue = OperationQueue()
    queue.maxConcurrentOperationCount = 3
    queue.qualityOfService = .userInitiated
    return queue
  }()

  /// "Choose more" answers waiting for the choice to reach the library.
  private var choosing: [Invoke] = []
  private var observing = false

  /// Photo numbers back to PhotoKit's identifiers.
  private var identifiers: [Int64: String] = [:]
  private let identifiersLock = NSLock()

  /// Only photos: On this phone has no videos.
  private static func onlyPhotos() -> PHFetchOptions {
    let options = PHFetchOptions()
    options.predicate = NSPredicate(format: "mediaType == %d", PHAssetMediaType.image.rawValue)
    return options
  }

  /// A photo's number: FNV-1a of its local identifier, kept to 53 bits so
  /// JavaScript holds it exactly, and never 0. The same on every launch.
  static func number(_ identifier: String) -> Int64 {
    var hash: UInt64 = 0xcbf2_9ce4_8422_2325
    for byte in identifier.utf8 {
      hash ^= UInt64(byte)
      hash = hash &* 0x100_0000_01b3
    }
    let n = Int64(hash & ((1 << 53) - 1))
    return n == 0 ? 1 : n
  }

  /// The photo numbered `id`. Every photo's number is learned again when it
  /// isn't known, as after a restart, or for a photo added since.
  private func asset(_ id: Int64) throws -> PHAsset {
    identifiersLock.lock()
    var identifier = identifiers[id]
    identifiersLock.unlock()
    if identifier == nil {
      var all: [Int64: String] = [:]
      PHAsset.fetchAssets(with: Self.onlyPhotos()).enumerateObjects { asset, _, _ in
        all[Self.number(asset.localIdentifier)] = asset.localIdentifier
      }
      identifiersLock.lock()
      identifiers.merge(all) { _, new in new }
      identifier = identifiers[id]
      identifiersLock.unlock()
    }
    guard let identifier,
      let asset = PHAsset.fetchAssets(withLocalIdentifiers: [identifier], options: nil).firstObject
    else {
      throw PhotosError("This photo isn't on the phone any more.")
    }
    return asset
  }

  /// When it was taken, in milliseconds; when it arrived for a photo that
  /// doesn't say.
  private static func takenMs(_ asset: PHAsset) -> Int64 {
    guard let date = asset.creationDate ?? asset.modificationDate else { return 0 }
    return Int64(date.timeIntervalSince1970 * 1000)
  }

  /// Runs `work` on the worker, resolving what it returns or rejecting what
  /// it throws.
  private func run<T: Encodable>(_ invoke: Invoke, _ work: @escaping () throws -> T) {
    worker.addOperation {
      do {
        invoke.resolve(try work())
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  /// "all", "some" (only the photos the person chose) or "none".
  private func currentAccess() -> String {
    switch PHPhotoLibrary.authorizationStatus(for: .readWrite) {
    case .authorized: return "all"
    case .limited: return "some"
    default: return "none"
    }
  }

  @objc public func access(_ invoke: Invoke) {
    invoke.resolve(AccessReply(access: currentAccess()))
  }

  /// Asks, or with chosen photos only, lets the person choose again.
  @objc public func requestAccess(_ invoke: Invoke) {
    if PHPhotoLibrary.authorizationStatus(for: .readWrite) == .limited {
      DispatchQueue.main.async {
        guard let presenter = self.manager.viewController else {
          invoke.resolve(AccessReply(access: self.currentAccess()))
          return
        }
        if !self.observing {
          PHPhotoLibrary.shared().register(self)
          self.observing = true
        }
        // The choice reaches the library a moment after the picker closes, so
        // the photos are read again only then: on the library's change, or
        // after a second without one (nothing was changed).
        PHPhotoLibrary.shared().presentLimitedLibraryPicker(from: presenter) { _ in
          DispatchQueue.main.async {
            self.choosing.append(invoke)
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) { self.answerChoosing() }
          }
        }
      }
    } else {
      // Once refused, iOS doesn't ask again: this answers "none" at once.
      PHPhotoLibrary.requestAuthorization(for: .readWrite) { _ in
        invoke.resolve(AccessReply(access: self.currentAccess()))
      }
    }
  }

  func photoLibraryDidChange(_ changeInstance: PHChange) {
    DispatchQueue.main.async { self.answerChoosing() }
  }

  /// Answers "Choose more", once. Main thread only.
  private func answerChoosing() {
    let waiting = choosing
    choosing = []
    for invoke in waiting {
      invoke.resolve(AccessReply(access: currentAccess()))
    }
  }

  /// The person's albums and the smart albums worth browsing, with their
  /// photo counts, largest first, as on Android. Empty ones are left out.
  @objc public func albums(_ invoke: Invoke) {
    run(invoke) {
      var albums: [Album] = []
      let photos = Self.onlyPhotos()
      let add = { (collection: PHAssetCollection) in
        let count = PHAsset.fetchAssets(in: collection, options: photos).count
        if count > 0 {
          albums.append(
            Album(id: collection.localIdentifier, name: collection.localizedTitle ?? "", count: count))
        }
      }
      let smart: [PHAssetCollectionSubtype] = [
        .smartAlbumFavorites, .smartAlbumSelfPortraits, .smartAlbumScreenshots,
        .smartAlbumDepthEffect, .smartAlbumPanoramas, .smartAlbumLivePhotos, .smartAlbumRAW,
      ]
      for subtype in smart {
        PHAssetCollection.fetchAssetCollections(with: .smartAlbum, subtype: subtype, options: nil)
          .enumerateObjects { collection, _, _ in add(collection) }
      }
      PHAssetCollection.fetchAssetCollections(with: .album, subtype: .any, options: nil)
        .enumerateObjects { collection, _, _ in add(collection) }
      return AlbumsReply(albums: albums.sorted { $0.count > $1.count })
    }
  }

  /**
   * The photos, newest first, each `[id, takenMs, modified, width, height,
   * raw]`: compact, as a phone can hold tens of thousands. `modified`
   * changes when the photo does, so a cached thumbnail knows it is stale.
   * PhotoKit's sizes are already upright.
   */
  @objc public func photos(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PhotosArgs.self)
    run(invoke) {
      let result: PHFetchResult<PHAsset>
      if let album = args.album {
        guard
          let collection = PHAssetCollection.fetchAssetCollections(
            withLocalIdentifiers: [album], options: nil
          ).firstObject
        else {
          throw PhotosError("That album isn't on the phone any more.")
        }
        result = PHAsset.fetchAssets(in: collection, options: Self.onlyPhotos())
      } else {
        result = PHAsset.fetchAssets(with: Self.onlyPhotos())
      }
      // Raws (ProRAW, imported DNGs) are the RAW smart album's: asking each
      // photo for its files would take seconds on a large library.
      var raws = Set<String>()
      PHAssetCollection.fetchAssetCollections(with: .smartAlbum, subtype: .smartAlbumRAW, options: nil)
        .enumerateObjects { collection, _, _ in
          PHAsset.fetchAssets(in: collection, options: nil).enumerateObjects { asset, _, _ in
            raws.insert(asset.localIdentifier)
          }
        }
      var rows: [[Int64]] = []
      rows.reserveCapacity(result.count)
      var seen: [Int64: String] = [:]
      result.enumerateObjects { asset, _, _ in
        let id = Self.number(asset.localIdentifier)
        seen[id] = asset.localIdentifier
        let modified = (asset.modificationDate ?? asset.creationDate)?.timeIntervalSince1970 ?? 0
        rows.append([
          id,
          Self.takenMs(asset),
          Int64(modified),
          Int64(asset.pixelWidth),
          Int64(asset.pixelHeight),
          raws.contains(asset.localIdentifier) ? 1 : 0,
        ])
      }
      self.identifiersLock.lock()
      self.identifiers.merge(seen) { _, new in new }
      self.identifiersLock.unlock()
      rows.sort { $0[1] != $1[1] ? $0[1] > $1[1] : $0[0] > $1[0] }
      return PhotosReply(photos: rows)
    }
  }

  /// A JPEG of photo `id`, upright, no larger than `size`, written to `path`.
  @objc public func image(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ImageArgs.self)
    run(invoke) {
      let asset = try self.asset(args.id)
      let options = PHImageRequestOptions()
      options.isSynchronous = true
      options.deliveryMode = .highQualityFormat
      options.resizeMode = .exact
      // An iCloud photo is fetched rather than left blank.
      options.isNetworkAccessAllowed = true
      var image: UIImage?
      PHImageManager.default().requestImage(
        for: asset, targetSize: CGSize(width: args.size, height: args.size),
        contentMode: .aspectFit, options: options
      ) { result, _ in image = result }
      guard let image, let jpeg = Self.upright(image).jpegData(compressionQuality: 0.85) else {
        throw PhotosError("Couldn't make this photo's image.")
      }
      try Self.write(jpeg, to: args.path)
      return Done()
    }
  }

  /**
   * The photo to edit, written to `path`: its own bytes when it is a JPEG,
   * PNG or WebP no larger than `maxEdge`, so nothing is lost; otherwise
   * (HEIC, a DNG, a 48 MP photo) decoded by iOS, upright, down to `maxEdge`
   * and saved as a high-quality JPEG. Resolves the photo's name and capture
   * time, and whether its bytes were copied as they are.
   */
  @objc public func original(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(OriginalArgs.self)
    run(invoke) {
      let asset = try self.asset(args.id)
      let (data, type) = try Self.imageData(asset)
      let plain = [UTType.jpeg, .png, .webP].contains { type?.conforms(to: $0) ?? false }
      let longest = max(asset.pixelWidth, asset.pixelHeight)
      let copied = plain && longest > 0 && longest <= args.maxEdge
      if copied {
        try Self.write(data, to: args.path)
      } else {
        try Self.write(try Self.decoded(data, maxEdge: args.maxEdge, quality: 0.95), to: args.path)
      }
      return OriginalReply(name: Self.name(asset), takenMs: Self.takenMs(asset), copied: copied)
    }
  }

  /**
   * Photo `id`'s file as the Photos app shows it (an edit made there
   * included), location and all, written to `path`: what Add to library
   * sends. Resolves its name, capture time and type.
   */
  @objc public func file(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(FileArgs.self)
    run(invoke) {
      let asset = try self.asset(args.id)
      let resources = PHAssetResource.assetResources(for: asset)
      guard
        let resource = resources.first(where: { $0.type == .fullSizePhoto })
          ?? resources.first(where: { $0.type == .photo })
      else {
        throw PhotosError("Couldn't read this photo.")
      }
      let target = URL(fileURLWithPath: args.path)
      let partial = URL(fileURLWithPath: args.path + ".part")
      try FileManager.default.createDirectory(
        at: target.deletingLastPathComponent(), withIntermediateDirectories: true)
      try? FileManager.default.removeItem(at: partial)
      let options = PHAssetResourceRequestOptions()
      options.isNetworkAccessAllowed = true
      var failure: Error?
      let done = DispatchSemaphore(value: 0)
      PHAssetResourceManager.default().writeData(for: resource, toFile: partial, options: options) {
        error in
        failure = error
        done.signal()
      }
      done.wait()
      if let failure {
        try? FileManager.default.removeItem(at: partial)
        throw failure
      }
      try Self.moveIn(partial, to: target)
      let type = UTType(resource.uniformTypeIdentifier)
      // An edit made in Photos is FullSizeRender.jpg: named after the photo.
      var name = Self.name(asset)
      if resource.type == .fullSizePhoto, let ext = type?.preferredFilenameExtension {
        name = (name as NSString).deletingPathExtension + "." + ext
      }
      return FileReply(
        name: name, takenMs: Self.takenMs(asset),
        mime: type?.preferredMIMEType ?? "application/octet-stream")
    }
  }

  /// What a saved copy keeps of the original's EXIF.
  private static let copiedExif: [CFString] = [
    kCGImagePropertyExifDateTimeOriginal,
    kCGImagePropertyExifOffsetTimeOriginal,
    kCGImagePropertyExifSubsecTimeOriginal,
    kCGImagePropertyExifDateTimeDigitized,
    kCGImagePropertyExifLensMake,
    kCGImagePropertyExifLensModel,
    kCGImagePropertyExifFNumber,
    kCGImagePropertyExifExposureTime,
    kCGImagePropertyExifISOSpeedRatings,
    kCGImagePropertyExifFocalLength,
    kCGImagePropertyExifFocalLenIn35mmFilm,
    kCGImagePropertyExifFlash,
  ]

  /**
   * Copies photo `id`'s camera details, capture time and location into the
   * JPEG at `path`, marked upright, without encoding it again. `takenMs`
   * stands in for a capture time the original doesn't give.
   */
  @objc public func copyMetadata(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(MetadataArgs.self)
    run(invoke) {
      let asset = try self.asset(args.id)
      let (data, _) = try Self.imageData(asset)
      let properties =
        CGImageSourceCreateWithData(data as CFData, nil)
        .flatMap { CGImageSourceCopyPropertiesAtIndex($0, 0, nil) as? [CFString: Any] } ?? [:]
      let exif = properties[kCGImagePropertyExifDictionary] as? [CFString: Any] ?? [:]
      let tiff = properties[kCGImagePropertyTIFFDictionary] as? [CFString: Any] ?? [:]

      let metadata = CGImageMetadataCreateMutable()
      let set = { (dictionary: CFString, key: CFString, value: Any) in
        _ = CGImageMetadataSetValueMatchingImageProperty(metadata, dictionary, key, value as CFTypeRef)
      }
      for key in Self.copiedExif {
        if let value = exif[key] { set(kCGImagePropertyExifDictionary, key, value) }
      }
      if exif[kCGImagePropertyExifDateTimeOriginal] == nil {
        let format = DateFormatter()
        format.locale = Locale(identifier: "en_US_POSIX")
        format.dateFormat = "yyyy:MM:dd HH:mm:ss"
        let taken = Date(timeIntervalSince1970: Double(args.takenMs) / 1000)
        set(kCGImagePropertyExifDictionary, kCGImagePropertyExifDateTimeOriginal, format.string(from: taken))
      }
      for key in [kCGImagePropertyTIFFMake, kCGImagePropertyTIFFModel] {
        if let value = tiff[key] { set(kCGImagePropertyTIFFDictionary, key, value) }
      }
      set(kCGImagePropertyTIFFDictionary, kCGImagePropertyTIFFOrientation, 1)
      if let gps = properties[kCGImagePropertyGPSDictionary] as? [CFString: Any] {
        for (key, value) in gps { set(kCGImagePropertyGPSDictionary, key, value) }
      }

      let target = URL(fileURLWithPath: args.path)
      let partial = URL(fileURLWithPath: args.path + ".part")
      guard let source = CGImageSourceCreateWithURL(target as CFURL, nil),
        let destination = CGImageDestinationCreateWithURL(
          partial as CFURL, UTType.jpeg.identifier as CFString, 1, nil)
      else {
        throw PhotosError("Couldn't read the saved copy.")
      }
      let options: [CFString: Any] = [
        kCGImageDestinationMetadata: metadata,
        kCGImageDestinationMergeMetadata: true,
      ]
      var error: Unmanaged<CFError>?
      guard CGImageDestinationCopyImageSource(destination, source, options as CFDictionary, &error)
      else {
        try? FileManager.default.removeItem(at: partial)
        throw error?.takeRetainedValue() ?? PhotosError("Couldn't write the photo's details.")
      }
      try Self.moveIn(partial, to: target)
      return Done()
    }
  }

  // MARK: Helpers

  /// The photo's bytes as Photos shows it, and their type.
  private static func imageData(_ asset: PHAsset) throws -> (Data, UTType?) {
    let options = PHImageRequestOptions()
    options.isSynchronous = true
    options.version = .current
    options.deliveryMode = .highQualityFormat
    options.isNetworkAccessAllowed = true
    var bytes: Data?
    var type: String?
    PHImageManager.default().requestImageDataAndOrientation(for: asset, options: options) {
      data, uti, _, _ in
      bytes = data
      type = uti
    }
    guard let bytes else { throw PhotosError("Couldn't read this photo.") }
    return (bytes, type.flatMap { UTType($0) })
  }

  /// `data` decoded upright, down to `maxEdge` on its longer side, as a JPEG.
  private static func decoded(_ data: Data, maxEdge: Int, quality: Double) throws -> Data {
    guard let source = CGImageSourceCreateWithData(data as CFData, nil),
      let image = CGImageSourceCreateThumbnailAtIndex(
        source, 0,
        [
          kCGImageSourceCreateThumbnailFromImageAlways: true,
          kCGImageSourceCreateThumbnailWithTransform: true,
          kCGImageSourceThumbnailMaxPixelSize: maxEdge,
        ] as CFDictionary)
    else {
      throw PhotosError("Couldn't decode this photo.")
    }
    let out = NSMutableData()
    guard let eight = eightBit(image),
      let destination = CGImageDestinationCreateWithData(
        out, UTType.jpeg.identifier as CFString, 1, nil)
    else {
      throw PhotosError("Couldn't save the photo to edit.")
    }
    CGImageDestinationAddImage(
      destination, eight, [kCGImageDestinationLossyCompressionQuality: quality] as CFDictionary)
    guard CGImageDestinationFinalize(destination) else {
      throw PhotosError("Couldn't save the photo to edit.")
    }
    return out as Data
  }

  /// `image` at 8 bits a channel, which JPEG needs: an iPhone's HEIC decodes
  /// to 10. Its colours stay in their own space (Display P3 on an iPhone),
  /// which the JPEG carries as its profile; an HDR space goes to Display P3.
  private static func eightBit(_ image: CGImage) -> CGImage? {
    if image.bitsPerComponent == 8 { return image }
    let p3 = CGColorSpace(name: CGColorSpace.displayP3)!
    var spaces = [p3]
    if let own = image.colorSpace, own.model == .rgb, !CGColorSpaceUsesITUR_2100TF(own) {
      spaces.insert(own, at: 0)
    }
    for space in spaces {
      if let context = CGContext(
        data: nil, width: image.width, height: image.height, bitsPerComponent: 8,
        bytesPerRow: 0, space: space, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)
      {
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        if let drawn = context.makeImage() { return drawn }
      }
    }
    return nil
  }

  /// `image` drawn upright, for one PhotoKit hands over turned.
  private static func upright(_ image: UIImage) -> UIImage {
    if image.imageOrientation == .up { return image }
    let format = UIGraphicsImageRendererFormat()
    format.scale = image.scale
    return UIGraphicsImageRenderer(size: image.size, format: format).image { _ in
      image.draw(in: CGRect(origin: .zero, size: image.size))
    }
  }

  /// Its file name in the Photos library.
  private static func name(_ asset: PHAsset) -> String {
    let resources = PHAssetResource.assetResources(for: asset)
    return (resources.first(where: { $0.type == .photo }) ?? resources.first)?.originalFilename
      ?? "photo.jpg"
  }

  /// Written aside and moved in, so a reader never sees half a file.
  private static func write(_ data: Data, to path: String) throws {
    let target = URL(fileURLWithPath: path)
    let partial = URL(fileURLWithPath: path + ".part")
    try FileManager.default.createDirectory(
      at: target.deletingLastPathComponent(), withIntermediateDirectories: true)
    try data.write(to: partial)
    try moveIn(partial, to: target)
  }

  /// Renames `partial` over `target`, replacing what is there in one step.
  private static func moveIn(_ partial: URL, to target: URL) throws {
    if rename(partial.path, target.path) != 0 {
      try? FileManager.default.removeItem(at: partial)
      throw PhotosError("Couldn't save the image.")
    }
  }
}

@_cdecl("init_plugin_photos")
func initPlugin() -> Plugin {
  return PhotosPlugin()
}
