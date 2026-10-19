// `impulse`: drives the Impulse window this terminal belongs to (see
// ImpulseProtocol/ControlProtocol.swift). Bundled in the app and put on
// PATH inside Impulse terminals.

import Darwin
import Foundation
import ImpulseProtocol

let arguments = Array(CommandLine.arguments.dropFirst())
let environment = ProcessInfo.processInfo.environment
let isHook = arguments.first == "hook"

func fail(_ message: String, code: Int32 = 1) -> Never {
  FileHandle.standardError.write(Data((message + "\n").utf8))
  exit(code)
}

/// The app's version, from the bundle this tool is in
/// (Contents/Resources/bin/impulse); "dev" for a build outside one.
func appVersion() -> String {
  let executable = URL(fileURLWithPath: CommandLine.arguments[0]).resolvingSymlinksInPath()
  let plist = executable.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    .appendingPathComponent("Info.plist")
  guard let data = try? Data(contentsOf: plist),
    let info = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any],
    let version = info["CFBundleShortVersionString"] as? String
  else { return "dev" }
  return version
}

// `impulse mcp`: serve MCP on stdin/stdout until the agent closes stdin.
// Outside Impulse it still answers (with no tools), so a server registered
// for every project never shows up as broken.
if arguments.first == "mcp" {
  if arguments.count > 1, arguments[1] == "-h" || arguments[1] == "--help" {
    print("impulse mcp: an MCP server on standard input and output, for agents in Impulse terminals.")
    exit(0)
  }
  signal(SIGPIPE, SIG_IGN)
  let output = NSLock()
  let server = MCPServer.standard(
    environment: environment, cwd: FileManager.default.currentDirectoryPath, version: appVersion()
  ) { line in
    output.lock()
    defer { output.unlock() }
    FileHandle.standardOutput.write(line)
  }
  while let line = readLine(strippingNewline: true) {
    server.handle(line: Data(line.utf8))
  }
  server.finish()
  exit(0)
}

var request: ControlRequest
switch ControlProtocol.request(
  arguments: arguments, environment: environment,
  cwd: FileManager.default.currentDirectoryPath,
  stdin: { FileHandle.standardInput.readDataToEndOfFile() })
{
case .success(let value):
  request = value
case .failure(let error):
  let wantsHelp = error.isHelp || (arguments.first.map { ["-h", "--help", "help"].contains($0) } ?? true)
  if wantsHelp {
    print(error.message)
    exit(0)
  }
  fail(error.message, code: 2)
}

// Hooks are configured for an agent everywhere, but only mean something
// inside Impulse: stay silent and successful anywhere else.
guard let socketPath = environment[ControlProtocol.socketKey] else {
  if isHook { exit(0) }
  fail("impulse: not running inside an Impulse terminal (IMPULSE_SOCKET isn't set)")
}

/// The contents of `path` in a new temporary file (same name), for the
/// editor; up to its 10 MB limit.
func temporaryCopy(of path: String) -> String {
  guard let input = FileHandle(forReadingAtPath: path) else { fail("impulse open: couldn't read \(path)") }
  let limit = 10 * 1024 * 1024
  let data: Data
  do {
    data = try input.read(upToCount: limit + 1) ?? Data()
  } catch {
    fail("impulse open: couldn't read \(path)")
  }
  guard data.count <= limit else { fail("impulse open: \(path) holds more than 10 MB") }
  var template = Array((NSTemporaryDirectory() + "impulse-open-XXXXXX").utf8CString)
  let directory = template.withUnsafeMutableBufferPointer { buffer -> String? in
    mkdtemp(buffer.baseAddress).map { String(cString: $0) }
  }
  guard let directory else { fail("impulse open: couldn't make a temporary file") }
  let copy = (directory as NSString).appendingPathComponent((path as NSString).lastPathComponent)
  guard FileManager.default.createFile(atPath: copy, contents: data) else {
    fail("impulse open: couldn't make a temporary file")
  }
  return copy
}

// A pipe (`impulse open <(git show HEAD:x)`, fish's `psub`), a device, or
// anything in /dev can only be read here: in the app, /dev/fd/63 would be
// one of its own files, whatever this one is. What it holds opens from a
// temporary file instead.
if request.command == "open", let path = request.arguments["path"] {
  var info = stat()
  let type = stat(path, &info) == 0 ? info.st_mode & S_IFMT : S_IFREG
  var resolved = path
  if let real = realpath(path, nil) {
    resolved = String(cString: real)
    free(real)
  }
  let inDev = [path, resolved].contains { $0 == "/dev" || $0.hasPrefix("/dev/") }
  if inDev || (type != S_IFREG && type != S_IFDIR) {
    if request.wait { fail("impulse edit: \(path) can't be edited in place; impulse open shows a copy") }
    request.arguments["path"] = temporaryCopy(of: path)
  }
}

// `impulse tasks new --brief-file notes.md`: the brief travels in the
// request (the app may not be able to read the file, and it shouldn't be
// read twice).
if let path = request.arguments.removeValue(forKey: "briefFile") {
  guard let input = FileHandle(forReadingAtPath: path) else { fail("impulse tasks new: couldn't read \(path)") }
  let data: Data
  do {
    // Nil at the end of the file: an empty brief.
    data = try input.read(upToCount: ControlProtocol.briefLimit + 1) ?? Data()
  } catch {
    fail("impulse tasks new: couldn't read \(path)")
  }
  guard data.count <= ControlProtocol.briefLimit else { fail("impulse tasks new: \(path) holds more than 16 KB") }
  request.arguments["brief"] = String(decoding: data, as: UTF8.self)
}

// An `edit` (or a `tasks wait`) waits here until it completes.
let reply: ControlResponse
switch ControlClient(socketPath: socketPath).send(request) {
case .success(let answer):
  reply = answer
case .failure(.unreachable):
  if isHook { exit(0) }
  fail("impulse: couldn't reach Impulse (is it still running?)")
case .failure(.noAnswer):
  if isHook { exit(0) }
  fail("impulse: no answer from Impulse")
}
if let message = reply.message, !message.isEmpty {
  if reply.ok { print(message) } else { FileHandle.standardError.write(Data((message + "\n").utf8)) }
}
exit(reply.ok ? 0 : 1)
