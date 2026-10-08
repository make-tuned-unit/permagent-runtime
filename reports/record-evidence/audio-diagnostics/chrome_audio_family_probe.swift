// Read-only HAL inventory restricted to one exact Chrome app and verified descendants.
// No tap/device creation, capture, permission request, or property writes.
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

func belongsToChrome(_ executable: String, contents: [String]) -> Bool {
    let path = URL(fileURLWithPath: executable).standardizedFileURL.pathComponents
    return path.count > contents.count && Array(path.prefix(contents.count)) == contents
}

// Pure ownership predicate: both ancestry and canonical executable containment
// must hold for every intermediary. A lookalike bundle or unrelated Chrome
// instance is not included. Caller revalidates the complete identity chain.
func ownedChain(pid: pid_t, root: ProcessIdentity, contents: [String],
                lookup: (pid_t) -> ProcessIdentity?) -> [ProcessIdentity]? {
    guard belongsToChrome(root.executable, contents: contents) else { return nil }
    var currentPID = pid
    var seen = Set<pid_t>()
    var chain: [ProcessIdentity] = []
    for _ in 0..<64 {
        guard currentPID > 1, seen.insert(currentPID).inserted,
              let current = lookup(currentPID), current.pid == currentPID,
              belongsToChrome(current.executable, contents: contents) else { return nil }
        chain.append(current)
        if currentPID == root.pid { return current == root ? chain : nil }
        currentPID = current.parent
    }
    return nil
}

func verifiedRole(pid: pid_t, rootPID: pid_t) -> String {
    if pid == rootPID { return "chrome_root" }
    // Called only after ownership validation. Never output raw process arguments.
    let ps = Process(), pipe = Pipe()
    ps.executableURL = URL(fileURLWithPath: "/bin/ps")
    ps.arguments = ["-ww", "-p", String(pid), "-o", "args="]
    ps.standardOutput = pipe; ps.standardError = FileHandle.nullDevice
    do { try ps.run() } catch { return "owned_descendant" }
    let data = pipe.fileHandleForReading.readDataToEndOfFile()
    ps.waitUntilExit()
    guard ps.terminationStatus == 0 else { return "owned_descendant" }
    let args = String(data: data, encoding: .utf8)?.split(whereSeparator: { $0.isWhitespace }) ?? []
    if args.contains("--utility-sub-type=audio.mojom.AudioService") { return "audio_service" }
    if args.contains("--type=renderer") { return "renderer" }
    if args.contains("--type=gpu-process") { return "gpu_process" }
    if args.contains("--type=utility") { return "utility" }
    return "owned_descendant"
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

func translatedProcess(_ pid: pid_t) throws -> AudioObjectID {
    var selectedPID = pid
    var property = address(kAudioHardwarePropertyTranslatePIDToProcessObject)
    var object = AudioObjectID(kAudioObjectUnknown), size = UInt32(MemoryLayout<AudioObjectID>.size)
    let status = AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &property,
        UInt32(MemoryLayout<pid_t>.size), &selectedPID, &size, &object)
    guard status == noErr, size == MemoryLayout<AudioObjectID>.size else {
        throw ProbeError("HAL PID translation failed")
    }
    return object // kAudioObjectUnknown means this PID has no HAL process object.
}

func inspectOwned(object: AudioObjectID, pid: pid_t, root: ProcessIdentity,
                  contents: [String], defaultOutput: AudioObjectID) -> [String: Any]? {
    guard let chain = ownedChain(pid: pid, root: root, contents: contents, lookup: { try? identity($0) }) else { return nil }
    var row: [String: Any] = ["pid": pid, "role": verifiedRole(pid: pid, rootPID: root.pid),
                             "hasHALObject": object != kAudioObjectUnknown]
    if object != kAudioObjectUnknown {
        guard (try? scalar(object, kAudioProcessPropertyPID, pid_t(0))) == pid else { return nil }
        row["processObjectId"] = object
        row["isRunning"] = try? scalar(object, kAudioProcessPropertyIsRunning, UInt32(0))
        row["isRunningOutput"] = try? scalar(object, kAudioProcessPropertyIsRunningOutput, UInt32(0))
        if let devices = try? objects(object, kAudioProcessPropertyDevices, scope: kAudioObjectPropertyScopeOutput) {
            row["outputDevices"] = devices.map { device -> [String: Any] in
                var deviceRow: [String: Any] = ["objectId": device, "isDefaultOutput": device == defaultOutput]
                deviceRow["isAlive"] = try? scalar(device, kAudioDevicePropertyDeviceIsAlive, UInt32(0))
                deviceRow["isRunningSomewhere"] = try? scalar(device, kAudioDevicePropertyDeviceIsRunningSomewhere, UInt32(0))
                deviceRow["sampleRate"] = try? scalar(device, kAudioDevicePropertyNominalSampleRate, Float64(0))
                deviceRow["masterMute"] = try? scalar(device, kAudioDevicePropertyMute, UInt32(0), scope: kAudioObjectPropertyScopeOutput)
                deviceRow["masterVolume"] = try? scalar(device, kAudioDevicePropertyVolumeScalar, Float32(0), scope: kAudioObjectPropertyScopeOutput)
                return deviceRow
            }
        } else {
            row["outputDeviceQueryUnavailable"] = true
        }
    }
    // A reused or reparented PID must not leak another process's metadata.
    guard (object == kAudioObjectUnknown || (try? scalar(object, kAudioProcessPropertyPID, pid_t(0))) == pid),
          chain.allSatisfy({ (try? identity($0.pid)) == $0 }),
          ownedChain(pid: pid, root: root, contents: contents, lookup: { try? identity($0) }) == chain
    else { return nil }
    return row
}

func selfTest() {
    let contents = URL(fileURLWithPath: "/Applications/Google Chrome.app/Contents").pathComponents
    func process(_ pid: pid_t, _ parent: pid_t, _ executable: String, start: UInt64 = 1) -> ProcessIdentity {
        ProcessIdentity(pid: pid, parent: parent, executable: executable, startedSeconds: start, startedMicros: 0)
    }
    let root = process(100, 1, "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
    let helper = "/Applications/Google Chrome.app/Contents/Frameworks/Helper.app/Contents/MacOS/Helper"
    var table: [pid_t: ProcessIdentity] = [100: root,
        101: process(101, 100, helper), 102: process(102, 101, helper),
        103: process(103, 999, helper),
        104: process(104, 100, "/Applications/Other.app/Contents/MacOS/Other"),
        105: process(105, 100, "/Applications/Google Chrome.app.other/Contents/MacOS/Helper"),
        106: process(106, 107, helper), 107: process(107, 106, helper),
        108: process(108, 104, helper)]
    func chain(_ pid: pid_t) -> [ProcessIdentity]? {
        ownedChain(pid: pid, root: root, contents: contents, lookup: { table[$0] })
    }
    precondition(chain(100) == [root])
    precondition(chain(101)?.map { $0.pid } == [101, 100])
    precondition(chain(102)?.map { $0.pid } == [102, 101, 100])
    for pid in [pid_t(1), 103, 104, 105, 106, 108, 109] { precondition(chain(pid) == nil) }
    table[100] = process(100, 1, root.executable, start: 2)
    precondition(chain(100) == nil && chain(101) == nil) // PID reused under the same path.
    table[100] = root
    table[101] = process(999, 100, helper)
    precondition(chain(101) == nil) // Lookup identity must match requested PID.
    print("PASS root/direct/nested ownership, unrelated ancestry, bundle boundary, cycle, missing process, reused PID and lookup mismatch; no HAL queries")
}

func main() throws {
    let args = CommandLine.arguments
    if args.count == 2 && args[1] == "--self-test" { selfTest(); return }
    guard args.count == 3, args[1] == "--chrome-pid", let rootPID = pid_t(args[2]), rootPID > 1,
          let app = NSRunningApplication(processIdentifier: rootPID),
          app.bundleIdentifier == "com.google.Chrome", let bundle = app.bundleURL else {
        throw ProbeError("Usage: chrome-audio-family-probe --chrome-pid EXACT_CHROME_ROOT_PID")
    }
    let root = try identity(rootPID)
    let contents = bundle.resolvingSymlinksInPath().appendingPathComponent("Contents").pathComponents
    guard belongsToChrome(root.executable, contents: contents) else { throw ProbeError("Chrome root escaped its app bundle") }
    let system = AudioObjectID(kAudioObjectSystemObject)
    let defaultOutput = try scalar(system, kAudioHardwarePropertyDefaultOutputDevice, AudioObjectID(0))
    let rootObject = try translatedProcess(rootPID)
    guard let rootRow = inspectOwned(object: rootObject, pid: rootPID, root: root, contents: contents, defaultOutput: defaultOutput) else {
        throw ProbeError("Chrome root identity changed during inspection")
    }
    var rows = [rootRow]
    var included = Set<AudioObjectID>()
    if rootObject != kAudioObjectUnknown { included.insert(rootObject) }
    // Global HAL object discovery is internal only. Unowned process identifiers,
    // bundles, names, counts, and arguments are never printed or persisted.
    for object in try objects(system, kAudioHardwarePropertyProcessObjectList) {
        guard !included.contains(object),
              let pid = try? scalar(object, kAudioProcessPropertyPID, pid_t(0)), pid != rootPID,
              let row = inspectOwned(object: object, pid: pid, root: root, contents: contents, defaultOutput: defaultOutput)
        else { continue }
        included.insert(object)
        rows.append(row)
    }
    guard try identity(rootPID) == root else { throw ProbeError("Chrome root exited or changed during inventory") }
    let result: [String: Any] = ["method": "read-only HAL metadata for exact Chrome root and verified owned descendants; no capture",
        "chromePID": rootPID, "processes": rows, "tapsCreated": 0, "devicesCreated": 0,
        "deviceWrites": 0, "permissionRequests": 0, "unownedProcessesEmitted": 0]
    let data = try JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys])
    FileHandle.standardOutput.write(data); print("")
}
do { try main() }
catch { FileHandle.standardError.write(Data("ERROR \(error)\n".utf8)); exit(1) }
