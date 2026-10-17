#if RETRIEVER_TESTING && RETRIEVER_DESKTOP_TESTING
import AppKit
import SwiftUI
import Carbon

enum SmokeError:Error {case failed(String)}

// This executable opens the shipping views and delegates with an isolated library.
// Events below are app-local integration tests, not physical keyboard acceptance.
@MainActor final class DesktopLifecycle:ObservableObject {
    let model:WorkspaceModel
    let output:URL
    let fixtures:URL
    let resume:Bool
    var started=false
    var checks:[String]=[]
    init() {
        let env=ProcessInfo.processInfo.environment
        output=URL(fileURLWithPath:env["RETRIEVER_TEST_OUTPUT"]!)
        fixtures=URL(fileURLWithPath:env["RETRIEVER_FIXTURES"]!)
        resume=env["RETRIEVER_TEST_RESUME"]=="1"
        model=WorkspaceModel(state:URL(fileURLWithPath:env["RETRIEVER_TEST_STATE"]!),endpoint:NativeProfile.endpoint,engine:URL(fileURLWithPath:env["RETRIEVER_ENGINE"]!),expectedVersion:env["RETRIEVER_TEST_ENGINE_VERSION"] ?? "0.13.0-alpha.16-offline",usePreferences:false)
        model.healthTestProvider=try! HealthFixtures(url:URL(fileURLWithPath:env["RETRIEVER_HEALTH_FIXTURES"]!))
    }
    func require(_ value:Bool,_ description:String) throws {
        guard value else {throw SmokeError.failed(description)};checks.append(description)
    }
    func pause() async throws {try await Task.sleep(for:.milliseconds(450))}
    func capture(_ window:NSWindow,_ name:String) throws {
        guard let view=window.contentView,let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {throw SmokeError.failed("Window capture unavailable")}
        view.cacheDisplay(in:view.bounds,to:bitmap)
        try bitmap.representation(using:.png,properties:[:])!.write(to:output.appendingPathComponent(name+".png"))
    }
    func key(_ window:NSWindow,code:UInt16,characters:String,flags:NSEvent.ModifierFlags=[]) async throws {
        guard let event=NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:flags,timestamp:ProcessInfo.processInfo.systemUptime,windowNumber:window.windowNumber,context:nil,characters:characters,charactersIgnoringModifiers:characters,isARepeat:false,keyCode:code) else {throw SmokeError.failed("Key event unavailable")}
        NSApp.postEvent(event,atStart:false);try await pause()
    }
    func waitBurner(_ expected:String) async throws {
        for _ in 0..<200 {
            try await model.refreshBurner()
            let job=model.burnerData["current"] as? [String:Any] ?? [:]
            if job["state"] as? String==expected {return}
            if job["state"] as? String=="failed" {throw SmokeError.failed(String(describing:job["error"]))}
            try await Task.sleep(for:.milliseconds(50))
        };throw SmokeError.failed("Burner timeout: "+expected)
    }
    func run(_ delegate:AppDelegate) async {
        guard !started else{return};started=true
        var report:[String:Any]=["passed":false,"physical_input":false,"health_fixture_only":true,"resume":resume]
        do {
            delegate.model=model;await model.start();delegate.prepareQuick();try await pause()
            guard let window=delegate.mainWindow else {throw SmokeError.failed("Main window registration")}
            window.title="Retriever — Fixture preview";window.makeKeyAndOrderFront(nil);NSApp.activate(ignoringOtherApps:true)
            try require(window.isVisible,"Main window visible")
            try require(delegate.hotkeyRegistered,"Carbon global shortcut registered")
            try require(delegate.statusItem?.button?.image != nil,"Menu-bar icon created")
            try require(delegate.statusItem?.button?.image?.isTemplate==true,"Menu-bar icon adapts to the native light and dark appearance")
            if resume {
                let expectedFolder=ProcessInfo.processInfo.environment["RETRIEVER_TEST_EXPECT_FOLDER"]
                try require(model.tabs.count>=2 && (expectedFolder==nil ? model.section=="burner":model.location["path"] as? String==expectedFolder),"Saved tabs and the selected location restored after restart")
                try await model.refreshBurner()
                try require((model.burnerData["current"] as? [String:Any])?["state"] as? String=="complete","Verification receipt survives restart")
            } else {
                var config=model.configuration;var devices=config["devices"] as? [[String:Any]] ?? []
                devices[0]["roots"]=[fixtures.path];config["devices"]=devices;config["configured"]=true
                if var search=config["search"] as? [String:Any] {for role in ["embedding","relevance","vision","llm"] {if var settings=search[role] as? [String:Any] {settings["provider"]="disabled";search[role]=settings}};config["search"]=search}
                _ = try await model.request("config",body:config,revision:model.revision)
                _ = try await model.request("sync",body:[:])
                for _ in 0..<200 {if (try await model.request("status"))["syncing"] as? Bool != true {break};try await Task.sleep(for:.milliseconds(100))}
                try await model.refresh();try await model.loadFiles()
                try require(model.files.count>=9,"Generated library indexed")
                let before=model.tabs.count
                try await key(window,code:17,characters:"t",flags:.command)
                try require(model.tabs.count==before+1,"Command-T opens one tab")
                try await key(window,code:13,characters:"w",flags:.command)
                try require(model.tabs.count==before,"Command-W closes one tab")
                let folderTabs=model.tabs.count
                delegate.application(NSApp,open:[fixtures]);try await pause()
                try require(model.location["path"] as? String==fixtures.path && model.tabs.count==folderTabs,"Folder activation reuses the current explorer tab")
                let board=NSPasteboard.withUniqueName();defer{board.releaseGlobally()}
                board.writeObjects([fixtures as NSURL]);var serviceError:NSString?
                delegate.openFolders(board,userData:nil,error:&serviceError);try await pause()
                try require(serviceError==nil && model.location["path"] as? String==fixtures.path && model.tabs.count==folderTabs,"Finder service opens a folder without duplicating its tab")
                await model.newTab()
                model.message="Disk Capacity · complete";await model.openBurner();try await model.refreshBurner()
                try require(model.message.hasPrefix("Disk Burner ·"),"Dashboard footer follows Disk Burner")
                let sample=try await model.request("burner/sample",body:[:]);try await model.inspectBurner(sample["path"] as! String);try await waitBurner("ready")
                let inspection=model.burnerData["inspection"] as! [String:Any]
                _ = try await model.request("burner/start",body:["inspection_id":inspection["id"]!,"target_id":"sandbox-8"])
                try await waitBurner("complete")
                let job=model.burnerData["current"] as! [String:Any];let image=job["image"] as! [String:Any]
                try require(job["verified_sha256"] as? String==image["sha256"] as? String,"Sample source and read-back SHA-256 match")
            }
            try await pause();try capture(window,"mac-burner-visible")
            delegate.statusItem?.button?.performClick(nil);try await pause()
            guard let overlay=delegate.overlay else {throw SmokeError.failed("Missing overlay")}
            try require(overlay.panel.isVisible && overlay.model.quickActive,"Menu-bar button opens overlay")
            overlay.model.query="Sequoia";await overlay.model.submitSearch();try await pause()
            try require(overlay.model.files.first?.name=="budget.xlsx","Overlay search finds fixture spreadsheet")
            try capture(overlay.panel,"mac-overlay-visible")
            try await key(overlay.panel,code:53,characters:"\u{1b}")
            try require(!overlay.panel.isVisible && !overlay.model.quickActive,"Escape dismisses overlay")
            // Exercise the installed Carbon handler without posting system keyboard input.
            var event:EventRef?
            let created=CreateEvent(nil,OSType(kEventClassKeyboard),UInt32(kEventHotKeyPressed),0,EventAttributes(0),&event)
            try require(created==noErr,"Carbon event created")
            if let event {var identifier=EventHotKeyID(signature:0x52545256,id:1);SetEventParameter(event,EventParamName(kEventParamDirectObject),EventParamType(typeEventHotKeyID),MemoryLayout<EventHotKeyID>.size,&identifier);let result=SendEventToEventTarget(event,GetApplicationEventTarget());ReleaseEvent(event);try require(result==noErr,"Registered Carbon handler accepts shortcut event")}
            try await pause();try require(overlay.panel.isVisible,"Shortcut handler opens overlay")
            delegate.showMain();try await pause();try require(window.isVisible && !overlay.panel.isVisible,"Open Retriever restores main window")
            delegate.showDriveHealth();try await pause();try require(model.section=="health" && (model.healthTestProvider?.requests ?? 0)>0,"Menu-bar health action uses injected fixtures")
            await model.openBurner();try await model.refreshBurner();model.rememberTabs()
            window.performClose(nil);try await pause();try require(!window.isVisible,"Close hides main window and retains menu bar")
            _ = delegate.applicationShouldHandleReopen(NSApp,hasVisibleWindows:false);try await pause();try require(window.isVisible,"Reopen restores main window")
            report["passed"]=true;report["tabs"]=model.tabs.count
        } catch {report["error"]=String(describing:error)}
        report["checks"]=checks;report["count"]=checks.count
        try? JSONSerialization.data(withJSONObject:report,options:[.prettyPrinted,.sortedKeys]).write(to:output.appendingPathComponent(resume ? "mac-lifecycle-resume.json":"mac-lifecycle.json"))
        if ProcessInfo.processInfo.environment["RETRIEVER_TEST_STAY_OPEN"] != "1" {model.rememberTabs();model.stopTestCoordinator();delegate.quit()}
    }
}

@main struct FixtureDesktopApp:App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var delegate
    @StateObject private var test=DesktopLifecycle()
    var body:some Scene {
        WindowGroup("Retriever — Fixture preview") {
            WorkspaceView(model:test.model).frame(minWidth:800,minHeight:500).background(MainWindowRegistration(delegate:delegate)).task {await test.run(delegate)}
        }.defaultSize(width:1280,height:820)
        .commands {CommandGroup(after:.newItem) {
            Button("New tab"){Task{await test.model.newTab()}}.keyboardShortcut("t")
            Button("Close tab"){Task{await test.model.closeTab(test.model.activeTabID)}}.keyboardShortcut("w")
            Button("Quick Search"){delegate.toggleQuick()}.keyboardShortcut(.space,modifiers:[.control,.option])
        }}
    }
}
#endif
