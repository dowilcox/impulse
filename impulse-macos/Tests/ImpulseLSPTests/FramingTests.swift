// Tests for the LSP base-protocol frame parser (Content-Length framing),
// covering the behaviors ported from `reader_task` in lsp.rs.
#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseLSP

  struct FramingTests {
    private func frame(_ body: String) -> [UInt8] {
      Array("Content-Length: \(body.utf8.count)\r\n\r\n\(body)".utf8)
    }

    @Test func singleMessage() {
      let parser = FrameParser()
      let messages = parser.feed(frame("{\"a\":1}"))
      #expect(messages == [Array("{\"a\":1}".utf8)])
      #expect(!parser.failed)
    }

    @Test func messageSplitAcrossArbitraryChunkBoundaries() {
      let bytes = frame("{\"hello\":\"world\"}")
      for chunkSize in [1, 2, 3, 5, 7, 11, 16] {
        let parser = FrameParser()
        var out: [[UInt8]] = []
        var index = 0
        while index < bytes.count {
          let end = min(index + chunkSize, bytes.count)
          out += parser.feed(Array(bytes[index..<end]))
          index = end
        }
        #expect(out == [Array("{\"hello\":\"world\"}".utf8)], "chunkSize=\(chunkSize)")
        #expect(!parser.failed)
      }
    }

    @Test func splitInsideHeader() {
      let parser = FrameParser()
      var out = parser.feed(Array("Content-Le".utf8))
      out += parser.feed(Array("ngth: 4\r".utf8))
      out += parser.feed(Array("\n\r\nab".utf8))
      out += parser.feed(Array("cd".utf8))
      #expect(out == [Array("abcd".utf8)])
      #expect(!parser.failed)
    }

    @Test func twoMessagesCoalescedInOneChunk() {
      let parser = FrameParser()
      let messages = parser.feed(frame("first") + frame("second-msg"))
      #expect(messages == [Array("first".utf8), Array("second-msg".utf8)])
      #expect(!parser.failed)
    }

    @Test func largePayload() {
      let body = String(repeating: "x", count: 1024 * 1024)
      let bytes = frame(body)
      let parser = FrameParser()
      var out: [[UInt8]] = []
      var index = 0
      let chunk = 64 * 1024
      while index < bytes.count {
        let end = min(index + chunk, bytes.count)
        out += parser.feed(Array(bytes[index..<end]))
        index = end
      }
      #expect(out.count == 1)
      #expect(out[0].count == 1024 * 1024)
      #expect(out[0].first == UInt8(ascii: "x"))
      #expect(out[0].last == UInt8(ascii: "x"))
      #expect(!parser.failed)
    }

    // lsp.rs matches "Content-Length: " exactly (case-sensitive): a
    // lowercase header is ignored, so that message's headers end with
    // length 0 and parsing continues with the next bytes as headers.
    @Test func headerMatchingIsCaseSensitive() {
      let parser = FrameParser()
      let messages = parser.feed(
        Array("content-length: 5\r\n\r\nContent-Length: 2\r\n\r\nok".utf8))
      #expect(messages == [Array("ok".utf8)])
      #expect(!parser.failed)
    }

    @Test func duplicateContentLengthKeepsFirstValue() {
      let parser = FrameParser()
      let messages = parser.feed(
        Array("Content-Length: 2\r\nContent-Length: 5\r\n\r\nok".utf8))
      #expect(messages == [Array("ok".utf8)])
      #expect(!parser.failed)
    }

    @Test func unknownHeadersAndMissingLengthAreSkipped() {
      let parser = FrameParser()
      let messages = parser.feed(
        Array("Foo: bar\r\n\r\nContent-Length: 2\r\n\r\nok".utf8))
      #expect(messages == [Array("ok".utf8)])
      #expect(!parser.failed)
    }

    @Test func overlongHeaderLineDropsConnection() {
      let parser = FrameParser()
      var out: [[UInt8]] = []
      // 3 chunks of 3000 bytes with no newline -> exceeds the 8192 bound.
      for _ in 0..<3 {
        out += parser.feed([UInt8](repeating: UInt8(ascii: "a"), count: 3000))
      }
      #expect(out.isEmpty)
      #expect(parser.failed)
      // Once failed, further input yields nothing.
      #expect(parser.feed(frame("ok")).isEmpty)
    }

    @Test func tooManyHeadersDropsConnection() {
      let parser = FrameParser()
      var input = ""
      for i in 0..<33 {
        input += "X-Header-\(i): 1\r\n"
      }
      input += "\r\n"
      let messages = parser.feed(Array(input.utf8))
      #expect(messages.isEmpty)
      #expect(parser.failed)
    }

    @Test func oversizedMessageIsSkippedAndDrained() {
      let oversized = FrameParser.maxMessageSize + 1
      let parser = FrameParser()
      var out = parser.feed(Array("Content-Length: \(oversized)\r\n\r\n".utf8))
      // Drain the oversized body in 1 MB chunks.
      var remaining = oversized
      let chunk = [UInt8](repeating: 0, count: 1024 * 1024)
      while remaining > 0 {
        let take = min(remaining, chunk.count)
        out += parser.feed(chunk[0..<take])
        remaining -= take
      }
      #expect(out.isEmpty)
      #expect(!parser.failed)
      // The stream stays in sync: the next message parses normally.
      #expect(parser.feed(frame("ok")) == [Array("ok".utf8)])
    }
  }
#endif
