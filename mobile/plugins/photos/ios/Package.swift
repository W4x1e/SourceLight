// swift-tools-version:5.5

import PackageDescription

// The iOS side of the photos plugin (PhotoKit). Built by Tauri's plugin build
// through swift-rs; `.tauri/tauri-api` is copied in by that build.
let package = Package(
  name: "tauri-plugin-photos",
  platforms: [
    .iOS(.v15)
  ],
  products: [
    .library(
      name: "tauri-plugin-photos",
      type: .static,
      targets: ["tauri-plugin-photos"])
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-photos",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources")
  ]
)
