// Test-only stand-in for Microsoft Teams: Meetily's capture picks windows by Teams' bundle id
// (com.microsoft.teams2, see assistant/vision/capture.rs) and title, so this window is captured
// like a real Teams meeting window (a meeting title, none of the main window's sections).
// The slide changes every 4 s so the change detector keeps frames.
import AppKit

let app = NSApplication.shared
app.setActivationPolicy(.regular)
let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 900, height: 600),
                      styleMask: [.titled, .closable], backing: .buffered, defer: false)
window.title = "Storage roadmap review | Microsoft Teams"
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
window.makeKeyAndOrderFront(nil)
app.activate(ignoringOtherApps: true)
app.run()
