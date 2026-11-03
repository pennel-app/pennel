import Foundation

@main
enum MeetingAnalysisBindingsProbe {
    static func main() throws {
        precondition(CommandLine.arguments.count == 3, "Pass an owned output directory and the qualification fixture.")
        let output = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
        let profile = output.appendingPathComponent("analysis-abi-preview-\(UUID().uuidString)", isDirectory: true)
        let fixture = try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[2]))
        let record = try JSONSerialization.jsonObject(with: fixture) as! [String: Any]
        var request = record["context"] as! [String: Any]
        request.removeValue(forKey: "actual")
        request.removeValue(forKey: "calibration_sha256")
        let requestData = try JSONSerialization.data(withJSONObject: request, options: [.sortedKeys])
        let engine = try PennelEngine.newPreview(dataDir: profile.path)
        let result = try engine.meetingAnalysisAvailability(requestJson: String(decoding: requestData, as: UTF8.self))
        precondition(result.useCase == .manualFinishedSuggestions)
        precondition(!result.configured && !result.qualified && !result.available && result.reasonCode == "unconfigured")
        let receipt: [String: Any] = ["abi_runtime": "passed", "profile": "isolated-preview", "configured": result.configured,
                                      "qualified": result.qualified, "available": result.available, "reason_code": result.reasonCode]
        let data = try JSONSerialization.data(withJSONObject: receipt, options: [.sortedKeys])
        print(String(decoding: data, as: UTF8.self))
    }
}
