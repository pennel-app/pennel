// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "PeerNativeQA",
    platforms: [.macOS(.v13), .iOS(.v16)],
    products: [.library(name: "PeerNativeQA", targets: ["PeerNativeQA"])],
    targets: [
        .target(name: "PeerNativeQA", swiftSettings: [.define("PENNEL_PEER_TRANSPORT_QA")]),
        .testTarget(name: "PeerNativeQATests", dependencies: ["PeerNativeQA"]),
    ],
    swiftLanguageModes: [.v6]
)
