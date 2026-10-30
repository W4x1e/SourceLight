import CoreLocation
import Foundation
import SwiftRs
import Tauri

struct StartArgs: Decodable {
  /// Where to append the points: one `epochMillis,lat,lon,accuracy,altitude` line each.
  let file: String
  /// How often to keep a position.
  let intervalMs: Int64?
}

struct TrackStatus: Encodable {
  let recording: Bool
  let file: String?
  let points: Int
  let lastFixMs: Int64
  let lastAccuracy: Double
}

struct Permissions: Encodable { let location: String }

/// Records a GPS track while the screen is off (docs/MOBILE.md, "Shoot
/// track"), with CoreLocation. Location is asked for while in use: updates
/// started in the app carry on in the background (the `location` background
/// mode), with the blue indicator showing a recording is on, so "Always" is
/// never needed.
///
/// iOS has no interval to ask for: it sends positions as they come, and one
/// is kept every `intervalMs`, or sooner when it is much more accurate than
/// the last. If iOS ends the app, the recording ends with it; the file keeps
/// what was written.
class ShootTrackPlugin: Plugin, CLLocationManagerDelegate {
  /// Made, and only used, on the main thread: CoreLocation calls back on
  /// the thread that made it, which needs a run loop.
  private lazy var locations: CLLocationManager = {
    let manager = CLLocationManager()
    manager.delegate = self
    manager.desiredAccuracy = kCLLocationAccuracyBest
    manager.activityType = .otherNavigation
    manager.pausesLocationUpdatesAutomatically = false
    return manager
  }()
  /// Answers waiting for the person to answer iOS's prompt.
  private var asking: [Invoke] = []

  private var recording = false
  private var file: String?
  private var writer: FileHandle?
  private var intervalMs: Int64 = 10_000
  private var points = 0
  private var lastFixMs: Int64 = 0
  private var lastAccuracy: Double = 0

  /// The file last recorded into, for the status after a restart.
  private static let savedFileKey = "shootTrack.file"

  private func permission() -> String {
    switch locations.authorizationStatus {
    case .authorizedAlways, .authorizedWhenInUse: return "granted"
    case .notDetermined: return "prompt"
    default: return "denied"
    }
  }

  /// Asks for location while in use, showing iOS's prompt if it has not been
  /// answered, and answers as Android's base class does: `{location}`.
  @objc public override func requestPermissions(_ invoke: Invoke) {
    DispatchQueue.main.async {
      if self.locations.authorizationStatus == .notDetermined {
        self.asking.append(invoke)
        self.locations.requestWhenInUseAuthorization()
      } else {
        invoke.resolve(Permissions(location: self.permission()))
      }
    }
  }

  func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
    guard manager.authorizationStatus != .notDetermined else { return }
    let waiting = asking
    asking = []
    for invoke in waiting {
      invoke.resolve(Permissions(location: permission()))
    }
  }

  @objc public func start(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(StartArgs.self)
    DispatchQueue.main.async {
      guard self.permission() == "granted" else {
        invoke.reject("Location permission is needed to record a track.")
        return
      }
      do {
        let url = URL(fileURLWithPath: args.file)
        try FileManager.default.createDirectory(
          at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        if !FileManager.default.fileExists(atPath: args.file) {
          FileManager.default.createFile(atPath: args.file, contents: nil)
        }
        let handle = try FileHandle(forWritingTo: url)
        handle.seekToEndOfFile()
        try? self.writer?.close()
        self.writer = handle
      } catch {
        invoke.reject("Couldn't open the track file: \(error.localizedDescription)")
        return
      }
      if self.file != args.file {
        self.points = 0
        self.lastFixMs = 0
      }
      self.file = args.file
      self.intervalMs = max(args.intervalMs ?? 10_000, 1_000)
      UserDefaults.standard.set(args.file, forKey: Self.savedFileKey)
      self.locations.allowsBackgroundLocationUpdates = true
      self.locations.showsBackgroundLocationIndicator = true
      self.locations.startUpdatingLocation()
      self.recording = true
      invoke.resolve(self.status())
    }
  }

  @objc public func stop(_ invoke: Invoke) {
    DispatchQueue.main.async {
      self.locations.stopUpdatingLocation()
      self.locations.allowsBackgroundLocationUpdates = false
      try? self.writer?.close()
      self.writer = nil
      self.recording = false
      invoke.resolve(self.status())
    }
  }

  @objc public func status(_ invoke: Invoke) {
    DispatchQueue.main.async {
      invoke.resolve(self.status())
    }
  }

  private func status() -> TrackStatus {
    TrackStatus(
      recording: recording,
      file: file ?? UserDefaults.standard.string(forKey: Self.savedFileKey),
      points: points,
      lastFixMs: lastFixMs,
      lastAccuracy: lastAccuracy)
  }

  func locationManager(_ manager: CLLocationManager, didUpdateLocations fixes: [CLLocation]) {
    guard recording, let writer else { return }
    for fix in fixes where fix.horizontalAccuracy >= 0 {
      let ms = Int64(fix.timestamp.timeIntervalSince1970 * 1000)
      let due = ms - lastFixMs >= intervalMs
      let sharper = lastFixMs > 0 && fix.horizontalAccuracy < lastAccuracy / 2
      guard points == 0 || due || sharper else { continue }
      let altitude = fix.verticalAccuracy >= 0 ? "\(fix.altitude)" : ""
      let line =
        "\(ms),\(fix.coordinate.latitude),\(fix.coordinate.longitude),\(fix.horizontalAccuracy),\(altitude)\n"
      writer.write(Data(line.utf8))
      points += 1
      lastFixMs = ms
      lastAccuracy = fix.horizontalAccuracy
    }
  }

  func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
    // A fix that couldn't be had (no signal yet): iOS keeps trying.
  }
}

@_cdecl("init_plugin_shoot_track")
func initPlugin() -> Plugin {
  return ShootTrackPlugin()
}
