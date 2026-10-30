import Foundation
import Photos
import SwiftRs
import Tauri

struct SaveArgs: Decodable {
  /// The file to copy in.
  let path: String
  /// The name it goes in as.
  let name: String
  let mime: String
}

struct Saved: Encodable { let name: String }

struct GalleryError: LocalizedError {
  let message: String
  init(_ message: String) { self.message = message }
  var errorDescription: String? { message }
}

/// Saves photos sent from the desktop, and copies saved in the editor, into
/// the Photos library (docs/MOBILE.md, "Send to phone"). It needs only
/// add-only access, asked for the first time: no album, which would need
/// access to read the library too. Photos dates each by its EXIF.
class GalleryPlugin: Plugin {
  @objc public func save(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(SaveArgs.self)
    PHPhotoLibrary.requestAuthorization(for: .addOnly) { status in
      guard status == .authorized || status == .limited else {
        invoke.reject(
          "SourceLight isn't allowed to add photos to Photos. To change that, open Settings, then Apps, SourceLight, Photos."
        )
        return
      }
      PHPhotoLibrary.shared().performChanges {
        let options = PHAssetResourceCreationOptions()
        options.originalFilename = args.name
        PHAssetCreationRequest.forAsset().addResource(
          with: .photo, fileURL: URL(fileURLWithPath: args.path), options: options)
      } completionHandler: { done, error in
        if done {
          invoke.resolve(Saved(name: args.name))
        } else {
          invoke.reject(error?.localizedDescription ?? "Couldn't save the photo to Photos.")
        }
      }
    }
  }
}

@_cdecl("init_plugin_gallery")
func initPlugin() -> Plugin {
  return GalleryPlugin()
}
