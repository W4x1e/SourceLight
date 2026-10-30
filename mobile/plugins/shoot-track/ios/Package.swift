// swift-tools-version:5.5

import PackageDescription

// The iOS side of the shoot track plugin (CoreLocation). Built by Tauri's
// plugin build through swift-rs; `.tauri/tauri-api` is copied in by that build.
let package = Package(
  name: "tauri-plugin-shoot-track",
  platforms: [
    .iOS(.v15)
  ],
  products: [
    .library(
      name: "tauri-plugin-shoot-track",
      type: .static,
      targets: ["tauri-plugin-shoot-track"])
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-shoot-track",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources")
  ]
)
