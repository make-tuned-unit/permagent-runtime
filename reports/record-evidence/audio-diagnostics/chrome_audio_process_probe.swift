// Read-only HAL metadata for one verified Chrome AudioService process.
// No tap/device creation, capture, permission request, or device property writes.
import AppKit
import CoreAudio
import Darwin
import Foundation

struct ProbeError: Error, CustomStringConvertible {
    let description: String
    init(_ message: String) { description = message }
}
struct ProcessIdentity: Equatable {
    let pid: pid_t
    let parent: pid_t
    let executable: String
    let startedSeconds: UInt64
    let startedMicros: UInt64
}
func identity(_ pid: pid_t) throws -> ProcessIdentity {
    var info = proc_bsdinfo()
    let size = Int32(MemoryLayout<proc_bsdinfo>.size)
    guard proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, size) == size else {
        throw ProbeError("Selected process is absent or inaccessible")
    }
    var path = [CChar](repeating: 0, count: 4096)
    let count = path.withUnsafeMutableBufferPointer { proc_pidpath(pid, $0.baseAddress, UInt32($0.count)) }
    guard count > 0 else { throw ProbeError("Selected process executable is unavailable") }
    let executable = URL(fileURLWithPath: String(cString: path)).resolvingSymlinksInPath().path
    return ProcessIdentity(pid: pid, parent: pid_t(info.pbi_ppid), executable: executable,
        startedSeconds: info.pbi_start_tvsec, startedMicros: info.pbi_start_tvusec)
}
func ownedAudioService(rootPID: pid_t, servicePID: pid_t) throws -> (ProcessIdentity, ProcessIdentity) {
    guard rootPID > 1, servicePID > 1, rootPID != servicePID,
          let app = NSRunningApplication(processIdentifier: rootPID),
          app.bundleIdentifier == "com.google.Chrome", let bundle = app.bundleURL else {
        throw ProbeError("Require distinct exact Chrome root and AudioService PIDs")
    }
    let root = try identity(rootPID), service = try identity(servicePID)
    let contents = bundle.resolvingSymlinksInPath().appendingPathComponent("Contents").pathComponents
    func withinChrome(_ executable: String) -> Bool {
        let components = URL(fileURLWithPath: executable).pathComponents
        return components.count > contents.count && Array(components.prefix(contents.count)) == contents
    }
    guard withinChrome(root.executable) else { throw ProbeError("Chrome root escaped its app bundle") }
    var current = service, seen = Set<pid_t>()
    var foundRoot = false
    for _ in 0..<64 {
        guard seen.insert(current.pid).inserted, withinChrome(current.executable) else { break }
        if current.parent == rootPID { foundRoot = true; break }
        guard current.parent > 1 else { break }
        current = try identity(current.parent)
    }
    guard foundRoot else { throw ProbeError("Audio process is not an owned Chrome descendant") }
    // Parse only for the exact Chromium service tag. Never emit the command line,
    // profile arguments, URLs, paths, or other process arguments into evidence.
    let ps = Process(), output = Pipe()
    ps.executableURL = URL(fileURLWithPath: "/bin/ps")
    ps.arguments = ["-p", String(servicePID), "-o", "args="]
    ps.standardOutput = output; ps.standardError = FileHandle.nullDevice
    try ps.run()
    let data = output.fileHandleForReading.readDataToEndOfFile()
    ps.waitUntilExit()
    let arguments = String(data: data, encoding: .utf8)?.split(whereSeparator: { $0.isWhitespace }) ?? []
    guard ps.terminationStatus == 0, arguments.contains("--utility-sub-type=audio.mojom.AudioService") else {
        throw ProbeError("Selected descendant is not the exact Chrome AudioService")
    }
    guard try identity(rootPID) == root, try identity(servicePID) == service else {
        throw ProbeError("Selected process identity changed during validation")
    }
    return (root, service)
}
func address(_ selector: AudioObjectPropertySelector, _ scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
}
func scalar<T>(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector, _ initial: T,
               scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) throws -> T {
    var result = initial, property = address(selector, scope)
    var size = UInt32(MemoryLayout<T>.size)
    let status = withUnsafeMutablePointer(to: &result) {
        AudioObjectGetPropertyData(object, &property, 0, nil, &size, $0)
    }
    guard status == noErr, size == MemoryLayout<T>.size else {
        throw ProbeError("HAL property \(selector) unavailable (\(status))")
    }
    return result
}
func objects(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector,
             scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) throws -> [AudioObjectID] {
    var property = address(selector, scope), size: UInt32 = 0
    guard AudioObjectGetPropertyDataSize(object, &property, 0, nil, &size) == noErr,
          size % UInt32(MemoryLayout<AudioObjectID>.size) == 0, size <= 4096 else {
        throw ProbeError("HAL object-list size unavailable or excessive")
    }
    if size == 0 { return [] }
    var values = [AudioObjectID](repeating: 0, count: Int(size) / MemoryLayout<AudioObjectID>.size)
    let status = values.withUnsafeMutableBytes {
        AudioObjectGetPropertyData(object, &property, 0, nil, &size, $0.baseAddress!)
    }
    guard status == noErr else { throw ProbeError("HAL object-list read failed (\(status))") }
    return Array(values.prefix(Int(size) / MemoryLayout<AudioObjectID>.size))
}
func main() throws {
    let args = CommandLine.arguments
    guard args.count == 5, args[1] == "--chrome-pid", args[3] == "--audio-pid",
          let rootPID = pid_t(args[2]), var servicePID = pid_t(args[4]) else {
        throw ProbeError("Usage: chrome-audio-process-probe --chrome-pid PID --audio-pid PID")
    }
    let (root, service) = try ownedAudioService(rootPID: rootPID, servicePID: servicePID)
    var property = address(kAudioHardwarePropertyTranslatePIDToProcessObject)
    var object = AudioObjectID(kAudioObjectUnknown), size = UInt32(MemoryLayout<AudioObjectID>.size)
    let status = AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &property,
        UInt32(MemoryLayout<pid_t>.size), &servicePID, &size, &object)
    guard status == noErr, object != kAudioObjectUnknown,
          try scalar(object, kAudioProcessPropertyPID, pid_t(0)) == servicePID else {
        throw ProbeError("Selected Chrome AudioService has no matching HAL process object")
    }
    let devices = try objects(object, kAudioProcessPropertyDevices, scope: kAudioObjectPropertyScopeOutput)
    let defaultOutput = try scalar(AudioObjectID(kAudioObjectSystemObject), kAudioHardwarePropertyDefaultOutputDevice, AudioObjectID(0))
    let outputRows: [[String: Any]] = devices.map { device in
        var row: [String: Any] = ["objectId": device, "isDefaultOutput": device == defaultOutput]
        row["isAlive"] = try? scalar(device, kAudioDevicePropertyDeviceIsAlive, UInt32(0))
        row["isRunningSomewhere"] = try? scalar(device, kAudioDevicePropertyDeviceIsRunningSomewhere, UInt32(0))
        row["sampleRate"] = try? scalar(device, kAudioDevicePropertyNominalSampleRate, Float64(0))
        row["masterMute"] = try? scalar(device, kAudioDevicePropertyMute, UInt32(0), scope: kAudioObjectPropertyScopeOutput)
        row["masterVolume"] = try? scalar(device, kAudioDevicePropertyVolumeScalar, Float32(0), scope: kAudioObjectPropertyScopeOutput)
        return row
    }
    let running = try scalar(object, kAudioProcessPropertyIsRunning, UInt32(0))
    let runningOutput = try scalar(object, kAudioProcessPropertyIsRunningOutput, UInt32(0))
    guard try identity(rootPID) == root, try identity(servicePID) == service else {
        throw ProbeError("Selected process identity changed during HAL inspection")
    }
    let result: [String: Any] = ["method": "read-only exact Chrome AudioService HAL metadata; no capture",
        "chromePID": rootPID, "audioServicePID": servicePID, "processObjectId": object,
        "isRunning": running, "isRunningOutput": runningOutput, "outputDevices": outputRows,
        "tapsCreated": 0, "devicesCreated": 0, "deviceWrites": 0, "permissionRequests": 0]
    let data = try JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys])
    FileHandle.standardOutput.write(data); print("")
}
do { try main() }
catch { FileHandle.standardError.write(Data("ERROR \(error)\n".utf8)); exit(1) }
