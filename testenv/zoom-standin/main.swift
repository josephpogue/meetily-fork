// Test-only stand-in for Zoom: Meetily's capture picks windows by Zoom's bundle id
// (us.zoom.xos, see assistant/vision/capture.rs) and title, so this window is captured
// like a real Zoom meeting. The slide changes every 4 s so the change detector keeps frames.
import AppKit
import AVFoundation

let app = NSApplication.shared
app.setActivationPolicy(.regular)
let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 900, height: 600),
                      styleMask: [.titled, .closable], backing: .buffered, defer: false)
// `--home` is the app's home window: no call, so no microphone and no sound.
let home = CommandLine.arguments.contains("--home")
window.title = home ? "Zoom Workplace" : "Zoom Meeting"
let label = NSTextField(labelWithString: "")
label.font = .systemFont(ofSize: 72, weight: .bold)
label.frame = NSRect(x: 0, y: 250, width: 900, height: 100)
label.alignment = .center
window.contentView?.addSubview(label)
let colours: [NSColor] = [.systemBlue, .systemOrange, .systemGreen, .systemPurple]
var slide = 0
func show() {
  window.contentView?.wantsLayer = true
  window.contentView?.layer?.backgroundColor = colours[slide % colours.count].cgColor
  label.stringValue = "Slide \(slide + 1)"
  slide += 1
}
show()
Timer.scheduledTimer(withTimeInterval: 4, repeats: true) { _ in show() }
// A call in progress: open the default input so Core Audio reports this process as running input (the data is
// thrown away), and loop this app's own clip (speech, then about 4 s of silence) on the default output.
var engine: AVAudioEngine?
var player: AVAudioPlayer?
if !home {
  let e = AVAudioEngine()
  e.inputNode.installTap(onBus: 0, bufferSize: 4096, format: nil) { _, _ in }
  do { try e.start() } catch { FileHandle.standardError.write(Data("mic open failed: \(error)\n".utf8)) }
  engine = e
  if let url = Bundle.main.url(forResource: "zoom-call", withExtension: "wav"),
     let p = try? AVAudioPlayer(contentsOf: url) {
    p.numberOfLoops = -1
    p.play()
    player = p
  } else {
    FileHandle.standardError.write(Data("call clip missing from the bundle\n".utf8))
  }
}
window.makeKeyAndOrderFront(nil)
app.activate(ignoringOtherApps: true)
app.run()
