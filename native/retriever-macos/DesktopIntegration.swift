import AppKit
import Carbon
import SwiftUI

enum RetrieverMenuBarIcon {
    static func make() -> NSImage {
        // A separate template keeps the dog recognizable at menu-bar size. The
        // full-color application artwork would become a solid square as a mask.
        let image=NSImage(size:NSSize(width:18,height:18),flipped:false) {_ in
            NSColor.black.set()
            let outline=NSBezierPath(roundedRect:NSRect(x:1.5,y:1.5,width:15,height:15),xRadius:4,yRadius:4)
            outline.lineWidth=1.3;outline.stroke()
            for x in [CGFloat(5),CGFloat(11.5)] {
                NSBezierPath(roundedRect:NSRect(x:x,y:10,width:1.5,height:3),xRadius:0.75,yRadius:0.75).fill()
            }
            let nose=NSBezierPath()
            nose.move(to:NSPoint(x:6.8,y:8.5))
            nose.curve(to:NSPoint(x:11.2,y:8.5),controlPoint1:NSPoint(x:6.8,y:9.6),controlPoint2:NSPoint(x:11.2,y:9.6))
            nose.curve(to:NSPoint(x:9,y:6.8),controlPoint1:NSPoint(x:11.2,y:7.9),controlPoint2:NSPoint(x:9.8,y:6.8))
            nose.curve(to:NSPoint(x:6.8,y:8.5),controlPoint1:NSPoint(x:8.2,y:6.8),controlPoint2:NSPoint(x:6.8,y:7.9));nose.close();nose.fill()
            let smile=NSBezierPath();smile.lineWidth=1.25;smile.lineCapStyle = .round
            smile.move(to:NSPoint(x:3.8,y:6.3));smile.curve(to:NSPoint(x:9,y:6.4),controlPoint1:NSPoint(x:5.2,y:3.6),controlPoint2:NSPoint(x:8.1,y:3.8))
            smile.line(to:NSPoint(x:9,y:7));smile.move(to:NSPoint(x:9,y:6.4))
            smile.curve(to:NSPoint(x:14.2,y:6.3),controlPoint1:NSPoint(x:9.9,y:3.8),controlPoint2:NSPoint(x:12.8,y:3.6));smile.stroke()
            return true
        }
        image.isTemplate=true
        image.accessibilityDescription="Retriever"
        return image
    }
}

struct MainWindowRegistration:NSViewRepresentable {
    let delegate:AppDelegate
    func makeNSView(context:Context)->NSView {let view=NSView();DispatchQueue.main.async {if let window=view.window {delegate.mainWindow=window;window.delegate=delegate}};return view}
    func updateNSView(_ view:NSView,context:Context) {}
}

enum NativeProfile {
    static let isolated=ProcessInfo.processInfo.environment["RETRIEVER_PROFILE"] != nil || ProcessInfo.processInfo.environment["RETRIEVER_STATE"] != nil
    private static let bootstrap:([String:Any],String?) = {
        let saved=FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/Retriever Candidate/installation.json")
        guard !isolated,let url=FileManager.default.fileExists(atPath:saved.path) ? saved:Bundle.main.url(forResource:"coordinator",withExtension:"json") else{return ([:],nil)}
        do {
            guard let value=try JSONSerialization.jsonObject(with:Data(contentsOf:url)) as? [String:Any],
                  let state=value["state"] as? String,state.hasPrefix("/"),!state.contains("\0"),
                  let port=value["port"] as? Int,(1024...65535).contains(port) else {
                return ([:],"The saved library location is invalid. Restore coordinator.json before opening Retriever.")
            }
            if let endpoint=value["shared_endpoint"] as? String {
                guard let url=URL(string:endpoint),url.user==nil,url.password==nil,url.query==nil,url.fragment==nil,
                      url.path.isEmpty || url.path=="/",
                      url.scheme=="https" || (url.scheme=="http" && ["127.0.0.1","localhost","::1"].contains(url.host ?? "")),
                      let device=value["local_device"] as? String,device.range(of:"^[A-Za-z0-9_-]{1,64}$",options:.regularExpression) != nil else {
                    return ([:],"The shared coordinator or this Mac's device identity is invalid.")
                }
            }
            if url != saved {try FileManager.default.createDirectory(at:saved.deletingLastPathComponent(),withIntermediateDirectories:true,attributes:[.posixPermissions:0o700]);try JSONSerialization.data(withJSONObject:value).write(to:saved,options:.atomic)}
            return (value,nil)
        } catch {return ([:],"The saved library location could not be read: \(error.localizedDescription)")}
    }()
    static var startupError:String? {bootstrap.1}
    static var sharedEndpoint:String? {(bootstrap.0["shared_endpoint"] as? String)?.trimmingCharacters(in:CharacterSet(charactersIn:"/"))}
    static var sharedLocalDevice:String? {bootstrap.0["local_device"] as? String}
    static var sharedToken:String {(try? String(contentsOf:state.appendingPathComponent("shared-coordinator-token"),encoding:.utf8))?.trimmingCharacters(in:.whitespacesAndNewlines) ?? ""}
    static let name:String = {
        let raw=ProcessInfo.processInfo.environment["RETRIEVER_PROFILE"] ?? "state"
        return raw.range(of:"^[a-zA-Z0-9][a-zA-Z0-9_-]{0,63}$",options:.regularExpression) != nil ? raw : "state"
    }()
    static var state:URL {
        if let path=ProcessInfo.processInfo.environment["RETRIEVER_STATE"],path.hasPrefix("/"){return URL(fileURLWithPath:path,isDirectory:true)}
        if let path=bootstrap.0["state"] as? String {return URL(fileURLWithPath:path,isDirectory:true)}
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/Retriever Candidate/" + name)
    }
    static var endpoint:String {
        let port=Int(ProcessInfo.processInfo.environment["RETRIEVER_PORT"] ?? "") ?? (bootstrap.0["port"] as? Int) ?? 18780
        return "http://127.0.0.1:\((1024...65535).contains(port) ? port : 18780)"
    }
}

@MainActor final class AppDelegate:NSObject,NSApplicationDelegate,NSWindowDelegate {
    var model:WorkspaceModel?
    var pendingFolders:[URL]=[]
    var overlay:QuickOverlayController?
    var statusItem:NSStatusItem?
    var hotKey:EventHotKeyRef?
    var handler:EventHandlerRef?
    var tabKeyMonitor:Any?
    weak var mainWindow:NSWindow?
    private(set) var hotkeyRegistered=false
    func applicationDidFinishLaunching(_ notification:Notification) {
        NSApp.servicesProvider=self
        NSUpdateDynamicServices()
        tabKeyMonitor=NSEvent.addLocalMonitorForEvents(matching:.keyDown) {[weak self] event in
            guard let self,let model=self.model,NSApp.keyWindow === self.mainWindow,NSApp.keyWindow?.attachedSheet==nil else{return event}
            let flags=event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            if flags == .command,!(NSApp.keyWindow?.firstResponder is NSTextView) {
                if event.keyCode==8 {model.copyFiles();return nil}
                if event.keyCode==9 {model.pasteFiles();return nil}
            }
            if flags.contains(.command) && !flags.contains(.option) && [UInt16(17),UInt16(13)].contains(event.keyCode) {
                if event.keyCode==17 {Task {if flags.contains(.shift) {await model.reopenTab()}else{await model.newTab()}}}
                else {Task {await model.closeTab(model.activeTabID)}}
                return nil
            }
            if flags.contains(.control) && !flags.contains(.option) && event.keyCode==48 {Task {await model.cycleTab(flags.contains(.shift) ? -1:1)};return nil}
            return event
        }
        statusItem=NSStatusBar.system.statusItem(withLength:NSStatusItem.squareLength)
        statusItem?.button?.image=RetrieverMenuBarIcon.make()
        statusItem?.button?.toolTip="Retriever · Quick Search (⌃⌥Space)"
        statusItem?.button?.setAccessibilityLabel("Retriever Quick Search")
        statusItem?.button?.target=self;statusItem?.button?.action=#selector(statusClicked)
        statusItem?.button?.sendAction(on:[.leftMouseUp,.rightMouseUp])
        var spec=EventTypeSpec(eventClass:OSType(kEventClassKeyboard),eventKind:UInt32(kEventHotKeyPressed))
        let installed=InstallEventHandler(GetApplicationEventTarget(),{_,_,context in
            if let context {MainActor.assumeIsolated {Unmanaged<AppDelegate>.fromOpaque(context).takeUnretainedValue().toggleQuick()}}
            return noErr
        },1,&spec,Unmanaged.passUnretained(self).toOpaque(),&handler)
        let identifier=EventHotKeyID(signature:0x52545256,id:1)
        hotkeyRegistered=installed==noErr && RegisterEventHotKey(UInt32(kVK_Space),UInt32(controlKey|optionKey),identifier,GetApplicationEventTarget(),0,&hotKey)==noErr
        if !hotkeyRegistered && installed==noErr {
            hotkeyRegistered=RegisterEventHotKey(UInt32(kVK_ANSI_R),UInt32(controlKey|optionKey),identifier,GetApplicationEventTarget(),0,&hotKey)==noErr
            statusItem?.button?.toolTip=hotkeyRegistered ? "Retriever · Quick Search (⌃⌥R; Space is in use)" : "Retriever · Quick Search (shortcut unavailable)"
        }
        if NativeProfile.isolated {fputs("Retriever shortcut registered=\(hotkeyRegistered); menu-bar item=\(statusItem?.button?.image != nil)\n",stderr)}
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender:NSApplication)->Bool {false}
    func application(_ application:NSApplication,open urls:[URL]) {
        do {enqueueFolders(try FolderOpenRequest.directories(urls))}
        catch {model?.message=error.localizedDescription}
    }
    @objc func openFolders(_ pasteboard:NSPasteboard,userData:String?,error errorPointer:AutoreleasingUnsafeMutablePointer<NSString?>) {
        do {
            guard model?.canUseLocalFiles == true else {throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"Pair this Mac's device identity before opening local folders."])}
            let urls=pasteboard.readObjects(forClasses:[NSURL.self],options:[.urlReadingFileURLsOnly:true]) as? [URL] ?? []
            enqueueFolders(try FolderOpenRequest.directories(urls))
        } catch {errorPointer.pointee=error.localizedDescription as NSString}
    }
    private func enqueueFolders(_ urls:[URL]) {
        var seen=Set(pendingFolders.map{$0.standardizedFileURL.path})
        pendingFolders.append(contentsOf:urls.filter{seen.insert($0.standardizedFileURL.path).inserted}.prefix(max(0,16-pendingFolders.count)))
        drainFolders()
    }
    func drainFolders(){guard let model,model.tabsReady,!pendingFolders.isEmpty else{return};let urls=Array(pendingFolders.prefix(16));pendingFolders=[];showMain();Task{guard model.canUseLocalFiles else{model.message="Pair this Mac's device identity before opening local folders.";return};for url in urls{await model.browse(device:model.localDevice,path:url.path)}}}
    func windowShouldClose(_ sender:NSWindow)->Bool {if sender === mainWindow {model?.rememberTabs();sender.orderOut(nil);return false};return true}
    func applicationShouldHandleReopen(_ sender:NSApplication,hasVisibleWindows flag:Bool)->Bool {showMain();return true}
    func applicationWillTerminate(_ notification:Notification) {
        if let tabKeyMonitor {NSEvent.removeMonitor(tabKeyMonitor)}
        model?.rememberTabs();overlay?.dispose();if let hotKey {UnregisterEventHotKey(hotKey)};if let handler {RemoveEventHandler(handler)}
        if let statusItem {NSStatusBar.system.removeStatusItem(statusItem)}
    }
    @objc func statusClicked() {
        if NSApp.currentEvent?.type == .rightMouseUp,let button=statusItem?.button {
            let menu=NSMenu()
            for (title,action) in [("Quick Search",#selector(toggleQuick)),("Open Retriever",#selector(showMain)),("Drive health",#selector(showDriveHealth)),("Quit Retriever",#selector(quit))] {
                let item=NSMenuItem(title:title,action:action,keyEquivalent:"");item.target=self;menu.addItem(item)
            }
            menu.popUp(positioning:nil,at:NSPoint(x:0,y:button.bounds.minY),in:button)
        } else {toggleQuick()}
    }
    @objc func showDriveHealth(){showMain();Task {await model?.openDriveHealth()}}
    @objc func quit() {NSApp.terminate(nil)}
    @objc func showMain() {
        overlay?.hide(restoreFocus:false)
        (mainWindow ?? NSApp.windows.first(where:{$0.title.contains("Rust Candidate") && !($0 is NSPanel)}))?.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps:true)
    }
    func prepareQuick() {
        guard let model else {return}
        drainFolders()
        if overlay == nil {
            let quick=WorkspaceModel(state:model.state,endpoint:model.endpoint,engine:model.engine,expectedVersion:model.expectedVersion,quickSearch:true,usePreferences:false)
            quick.clientMode=model.clientMode;quick.token=model.token
            quick.revealInWorkspace={[weak self] file in self?.showMain();Task{await model.revealInRetriever(file)}}
            overlay=QuickOverlayController(model:quick,showExplorer:{[weak self] in self?.showMain()})
        }
    }
    @objc func toggleQuick() {
        guard let model else {return}
        prepareQuick()
        overlay?.model.clientMode=model.clientMode;overlay?.model.endpoint=model.endpoint;overlay?.model.token=model.token
        overlay?.toggle()
    }
}
