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

let request: ControlRequest
switch ControlProtocol.request(
  arguments: arguments, environment: environment,
  cwd: FileManager.default.currentDirectoryPath,
  stdin: { FileHandle.standardInput.readDataToEndOfFile() })
{
case .success(let value):
  request = value
case .failure(let error):
  let wantsHelp = arguments.first.map { ["-h", "--help", "help"].contains($0) } ?? true
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

let fd = socket(AF_UNIX, SOCK_STREAM, 0)
guard fd >= 0 else { fail("impulse: couldn't create a socket") }
var address = sockaddr_un()
address.sun_family = sa_family_t(AF_UNIX)
let pathBytes = Array(socketPath.utf8)
guard pathBytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
  fail("impulse: socket path too long")
}
withUnsafeMutableBytes(of: &address.sun_path) { buffer in
  for (index, byte) in pathBytes.enumerated() { buffer[index] = byte }
}
let connected = withUnsafePointer(to: &address) {
  $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
    connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
  }
}
guard connected == 0 else {
  if isHook { exit(0) }
  fail("impulse: couldn't reach Impulse (is it still running?)")
}

guard let line = try? ControlProtocol.encodeLine(request) else { fail("impulse: couldn't encode the request") }
_ = line.withUnsafeBytes { write(fd, $0.baseAddress, line.count) }

// Read one line of response (an `edit` waits here until its tab closes).
var response = Data()
var byte: UInt8 = 0
while read(fd, &byte, 1) == 1 {
  if byte == 0x0A { break }
  response.append(byte)
}
close(fd)

guard let reply = try? ControlProtocol.decode(ControlResponse.self, from: response) else {
  if isHook { exit(0) }
  fail("impulse: no answer from Impulse")
}
if let message = reply.message, !message.isEmpty {
  if reply.ok { print(message) } else { FileHandle.standardError.write(Data((message + "\n").utf8)) }
}
exit(reply.ok ? 0 : 1)
