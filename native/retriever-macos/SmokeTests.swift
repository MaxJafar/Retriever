#if RETRIEVER_TESTING
import AppKit
import SwiftUI

enum SmokeError: Error {case failed(String)}
@main struct NativeSmokeTests {
    @MainActor static var checks=0
    @MainActor static var renders=0
    @MainActor static func require(_ value:Bool,_ message:String) throws {if !value {throw SmokeError.failed(message)};checks+=1}
    @MainActor static func render(_ model:WorkspaceModel,_ destination:URL,quick:Bool=false,width:CGFloat=1176,inspector:Bool=false) async throws {
        // Render our own view offscreen: no screen capture, accessibility access,
        // installed app activation, or user's desktop content is involved.
        let view=NSHostingView(rootView:inspector ? AnyView(InspectorView(model:model).preferredColorScheme(.dark)) : quick ? AnyView(QuickOverlayView(model:model,dismiss:{},showExplorer:{})) : AnyView(WorkspaceView(model:model)))
        let height:CGFloat=inspector ? 538 : quick ? 580 : 760
        let window=NSWindow(contentRect:NSRect(x:0,y:0,width:width,height:height),styleMask:[.borderless],backing:.buffered,defer:false)
        window.contentView=view
        defer {window.contentView=nil}
        view.frame=NSRect(x:0,y:0,width:width,height:height)
        window.layoutIfNeeded();view.layoutSubtreeIfNeeded()
        try await Task.sleep(for:.milliseconds(300))
        guard let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else {throw SmokeError.failed("View bitmap unavailable")}
        view.cacheDisplay(in:view.bounds,to:bitmap)
        guard let png=bitmap.representation(using:.png,properties:[:]) else {throw SmokeError.failed("PNG rendering failed")}
        try png.write(to:destination)
        renders+=1
    }
    @MainActor static func renderComponent<V:View>(_ content:V,_ path:URL,width:CGFloat,height:CGFloat) async throws {
        let view=NSHostingView(rootView:content.preferredColorScheme(.dark).frame(maxWidth:.infinity,maxHeight:.infinity).background(Color(nsColor:.windowBackgroundColor)))
        let window=NSWindow(contentRect:NSRect(x:0,y:0,width:width,height:height),styleMask:[.borderless],backing:.buffered,defer:false)
        window.contentView=view;defer{window.contentView=nil};view.frame=NSRect(x:0,y:0,width:width,height:height);window.layoutIfNeeded();view.layoutSubtreeIfNeeded();try await Task.sleep(for:.milliseconds(300))
        guard let bitmap=view.bitmapImageRepForCachingDisplay(in:view.bounds) else{throw SmokeError.failed("Popover bitmap unavailable")};view.cacheDisplay(in:view.bounds,to:bitmap)
        guard let png=bitmap.representation(using:.png,properties:[:]) else{throw SmokeError.failed("Popover rendering failed")};try png.write(to:path);renders+=1
    }
    @MainActor static func main() async throws {
        let args=CommandLine.arguments
        guard args.count==7 else {throw SmokeError.failed("engine state fixtures output port version required")}
        NSApplication.shared.setActivationPolicy(.prohibited)
        let engine=URL(fileURLWithPath:args[1]),state=URL(fileURLWithPath:args[2]),fixtures=URL(fileURLWithPath:args[3]),output=URL(fileURLWithPath:args[4])
        let endpoint="http://127.0.0.1:\(args[5])"
        let model=WorkspaceModel(state:state,endpoint:endpoint,engine:engine,expectedVersion:args[6],usePreferences:false)
        defer {model.stopTestCoordinator()}
        await model.start()
        try require(model.ownedCoordinatorIdentifier != nil,"Fresh coordinator was not started: \(model.message)")
        let originalPID=model.ownedCoordinatorIdentifier
        var config=model.configuration
        var devices=config["devices"] as? [[String:Any]] ?? []
        try require(!devices.isEmpty,"Default local device missing")
        let device=devices[0]["id"] as? String ?? ""
        devices[0]["roots"]=[fixtures.path];config["devices"]=devices;config["configured"]=true
        _ = try await model.request("config",body:config,revision:model.revision)
        _ = try await model.request("sync",body:[:])
        var completed=false
        for _ in 0..<300 {
            try await Task.sleep(for:.milliseconds(100))
            let status=try await model.request("status")
            if status["syncing"] as? Bool != true {completed=true;break}
        }
        try require(completed,"Fixture crawl deadline exceeded")
        try await model.refresh();try await model.loadFiles()
        try require(model.files.count>=9,"Fixture catalogue was not populated: \(model.message)")
        try await render(model,output.appendingPathComponent("explorer.png"))
        try await render(model,output.appendingPathComponent("explorer-narrow.png"),width:820)
        await model.filterFiles("PDF")
        try require(model.files.count==1 && model.files[0].name=="astronomy.pdf","Native extension filtering failed")
        await model.navigate("files")
        await model.browse(device:device,path:fixtures.path)
        try require(model.backCount>0,"Browse history was not recorded")
        await model.back()
        try require(model.title=="All files","Back navigation failed")
        await model.forward()
        try require(model.title==fixtures.path,"Forward navigation failed")
        model.query="telescope Jupiter";await model.submitSearch()
        try require(model.files.first?.name=="astronomy.pdf","PDF content search failed: \(model.message)")
        try await render(model,output.appendingPathComponent("search.png"))
        model.selection=[model.files[0].id];await model.preview()
        try require(model.previewText.contains("Jupiter"),"Native text preview failed")
        model.showingPreview=false
        let originalTab=model.activeTabID
        await model.newTab()
        try require(model.tabs.count==2 && model.query.isEmpty,"New tab reused the previous search")
        let folderTab=model.activeTabID
        await model.browse(device:device,path:fixtures.path)
        model.gridMode=true;model.visibleColumns.insert("priority");model.sortKey="size";await model.resort()
        await model.switchTab(originalTab)
        try require(model.query=="telescope Jupiter" && model.files.first?.name=="astronomy.pdf","Tab switch lost its search")
        try require(!model.gridMode && model.sortKey=="name","View settings leaked between tabs")
        await model.switchTab(folderTab)
        try require(model.gridMode && model.visibleColumns.contains("priority") && model.backCount>0,"Folder tab lost its view or history")
        await model.closeTab(folderTab);try require(model.tabs.count==1,"Close tab failed")
        await model.reopenTab();try require(model.tabs.count==2 && model.gridMode,"Reopen did not restore tab state")
        let reopenedID=model.activeTabID;model.pinTab(reopenedID);await model.closeTab(reopenedID)
        try require(model.tabs.count==2,"Pinned tab was closed")
        model.pinTab(reopenedID);model.shiftTab(reopenedID,-1);model.rememberTabs()
        try require(model.tabs.first?.id==reopenedID,"Tab reorder failed")
        model.moveTab(reopenedID,before:originalTab)
        try require(model.tabs.first?.id==reopenedID,"Dragging before the following tab moved it after the target")
        let restored=WorkspaceModel(state:state,endpoint:endpoint,engine:engine,expectedVersion:model.expectedVersion,usePreferences:false)
        await restored.start()
        try require(restored.tabs.count==2 && restored.gridMode && restored.sortKey=="size","Tabs were not restored after restart")
        try require(restored.ownedCoordinatorIdentifier==nil,"Restoring tabs started a duplicate coordinator")
        await model.switchTab(originalTab)
        try await render(model,output.appendingPathComponent("tabs.png"))
        let countBefore=model.tabs.count
        await model.browse(device:device,path:fixtures.path)
        try require(model.activeTabID==reopenedID && model.tabs.count==countBefore,"Folder destination did not reuse its tab")
        await model.newTab(state:model.folderTabState(device,fixtures.path));try require(model.tabs.count==countBefore+1 && model.activeTabID != reopenedID,"Explicit folder tab was reused")
        await model.openAssistant();let utilityID=model.activeTabID;let utilityCount=model.tabs.count
        await model.openAssistant();try require(model.tabs.count==utilityCount,"Repeated utility click created another tab")
        await model.newTab();await model.openAssistant();try require(model.activeTabID==utilityID && model.tabs.count==utilityCount+1,"Utility destination did not reuse its tab")
        await model.switchTab(originalTab)


        model.query="Sequoia budget";await model.submitSearch();model.selection=[model.files[0].id];await model.inspect()
        try require(model.showingInspector && model.editingFile?.name=="budget.xlsx","Properties opened the wrong file")
        try await model.createProperty(name:"Project",kind:"select",options:["Sequoia","Moonstone"])
        try await model.createProperty(name:"Channels",kind:"multi_select",options:["Desktop","Web"])
        var propertyValues:[String:Any]=[:]
        for field in model.propertyDefinitions {let id=field["id"] as! String;propertyValues[id]=field["kind"] as? String=="select" ? "Sequoia" : ["Desktop","Web"] as Any}
        let firstRevision=model.metadata["revision"] as? Int ?? 0
        try await model.saveTypedProperties(owner:"Reviewer",tags:["Launch","Review"],status:"In review",priority:"high",starred:true,values:propertyValues,revision:firstRevision)
        model.selection=[model.files[0].id];await model.inspect()
        try await model.addComment("Ready for the workspace acceptance review.")
        try require(model.metadata["priority"] as? String=="high","Priority did not persist")
        try require((model.metadata["comments"] as? [[String:Any]])?.count==1,"Comment did not persist")
        try await render(model,output.appendingPathComponent("properties.png"),width:368,inspector:true)
        model.showingInspector=false
        let emptyField=try await model.request("workspace/property",body:["name":"Milestone","kind":"multi_select","options":[]])
        let fieldID=emptyField["id"] as! String
        let choice=try await model.addPropertyOption(fieldID,"Review")
        try require(choice=="Review","Inline option creation failed")
        let reused=try await model.addPropertyOption(fieldID,"review")
        try require(reused=="Review" && (model.propertyDefinitions.first{$0["id"] as? String==fieldID}?["options"] as? [String])==["Review"],"Case-insensitive choices duplicated")
        let cellFile=model.files[0];var cellMetadata=model.metadata
        var cellValues=cellMetadata["properties"] as? [String:Any] ?? [:];cellValues[fieldID]=["Review"];cellMetadata["properties"]=cellValues;cellMetadata["tags"]=["Launch","Review","Design"]
        cellMetadata=try await model.saveCell(cellFile,metadata:cellMetadata)
        let verified=try await model.request("workspace/inspect",body:["device":cellFile.device,"path":cellFile.path])
        try require((verified["properties"] as? [String:Any])?[fieldID] as? [String]==["Review"],"Inline multi-select did not persist")
        let reusable=try await model.request("workspace/properties")
        try require((reusable["tags"] as? [String])?.contains("Design")==true,"New tags were not available to other files")
        cellMetadata["tags"]=["Launch","Review"];_ = try await model.saveCell(cellFile,metadata:cellMetadata)
        let removed=try await model.request("workspace/inspect",body:["device":cellFile.device,"path":cellFile.path])
        try require((removed["tags"] as? [String])?.contains("Design")==false,"Removing a tag did not persist")
        model.visibleColumns.formUnion(["tags","priority","prop:"+fieldID]);try await render(model,output.appendingPathComponent("properties-table.png"));try await render(model,output.appendingPathComponent("properties-table-narrow.png"),width:820)
        try await renderComponent(ChoicePopover(selected:.constant(["Launch","Review"]),options:["Launch","Review","Design"],create:{$0}),output.appendingPathComponent("tag-popover.png"),width:308,height:330)
        try await renderComponent(SmartCollectionFilters(model:model),output.appendingPathComponent("filter-popover.png"),width:328,height:320)
        try await renderComponent(NewPropertyPopover(model:model),output.appendingPathComponent("new-property-popover.png"),width:298,height:360)
        await model.collectionFilters([["field":"tags","operator":"contains","value":"Launch"],["field":"priority","operator":"is","value":"high"]])
        try require(model.files.count==1 && model.files[0].name=="budget.xlsx","Smart collection filters failed")
        await model.saveView("Launch priorities")
        try require(model.savedViews.contains(where:{$0["name"] as? String=="Launch priorities"}),"Smart collection was not saved")
        await model.browse(device:device,path:fixtures.path);await model.saveView("Fixture folder")
        let folderView=model.savedViews.first(where:{$0["name"] as? String=="Fixture folder"}) ?? [:]
        await model.navigate("files");await model.openView(folderView)
        try require(model.browseRoute=="browse" && model.location["path"] as? String==fixtures.path,"Saved folder expanded to All files")
        await model.navigate("files");model.query="telescope Jupiter";await model.submitSearch()
        let quick=WorkspaceModel(state:state,endpoint:endpoint,engine:engine,expectedVersion:model.expectedVersion,quickSearch:true,usePreferences:false)
        await quick.start()
        try require(quick.ownedCoordinatorIdentifier==nil && model.ownedCoordinatorIdentifier==originalPID,"Coordinator was not reused")
        try require(quick.files.isEmpty && quick.total==0,"Empty quick search loaded the catalogue")
        quick.query="Sequoia budget";await quick.submitSearch()
        try require(quick.files.first?.name=="budget.xlsx","XLSX content search failed")
        try require(model.query=="telescope Jupiter","Quick search changed the main window query")
        try require(quick.selected?.name=="budget.xlsx","Quick search must select the first result")
        try await render(quick,output.appendingPathComponent("quick-search.png"),quick:true,width:940)
        quick.query="unmatchedquickquery";quick.queryChanged()
        try require(quick.selection.isEmpty && quick.files.isEmpty,"Typing left an old file available to Enter")
        quick.query="";quick.queryChanged()
        try require(quick.files.isEmpty && quick.total==0,"Clearing quick search left stale results")
        // Typing while the panel connects must survive its asynchronous startup.
        quick.query="Sequoia budget"
        await quick.start()
        try await Task.sleep(for:.milliseconds(850))
        try require(quick.files.first?.name=="budget.xlsx","Startup cancelled an early quick-search query")
        for (name,coverage) in [("shape.png","Image preview"),("sample.mp4","Video frame at 0:00 · audio not played"),("astronomy.pdf","PDF · first page")] {
            let reply=try await quick.request("workspace/preview",body:["device":model.devices[0].id,"path":fixtures.appendingPathComponent(name).path,"visual":true])
            try require((reply["image"] as? String)?.hasPrefix("data:image/jpeg;base64,")==true,"Missing visual preview for \(name)")
            try require(reply["coverage"] as? String==coverage,"Missing preview coverage for \(name)")
        }
        quick.query="";quick.queryChanged()
        do {try model.requireCompatible(["runtime":"rust","version":"older"]);throw SmokeError.failed("Older coordinator accepted")}
        catch let error as NSError where error.domain=="Retriever" && error.code==4 {}
        let protected=state.deletingLastPathComponent().appendingPathComponent("protected-state")
        try FileManager.default.createDirectory(at:protected,withIntermediateDirectories:true)
        try Data("unchanged legacy sentinel".utf8).write(to:protected.appendingPathComponent("index.sqlite"))
        let guarded=WorkspaceModel(state:protected,endpoint:endpoint,engine:engine,expectedVersion:model.expectedVersion,usePreferences:false)
        await guarded.start()
        try require(guarded.ownedCoordinatorIdentifier==nil && guarded.message.contains("installed index is protected"),"Legacy state guard failed")
        let queries=[("Kyoto temples","travel.txt"),("Moonstone","unicode.txt"),("support escalation","support.docx"),("procurement approval","launch.pptx"),("greenhouse tomatoes","garden.md")]
        var timings:[Double]=[]
        for (query,expected) in queries {
            model.query=query
            let begin=Date();await model.submitSearch();timings.append(Date().timeIntervalSince(begin)*1000)
            try require(model.files.first?.name==expected,"Unexpected top result for \(query): \(model.message)")
        }
        await model.browse(device:device,path:fixtures.path);await model.openDiskSpace()
        try require(model.browseRoute=="disk" && model.section=="disk","Disk tab did not open")
        model.location["root"]=fixtures.path;await model.startDiskScan()
        for _ in 0..<150 {if !model.diskRunning {break};try await Task.sleep(for:.milliseconds(100));try await model.refreshDisk()}
        try require(model.diskEntries.count>=9 && model.diskScan["state"] as? String=="complete","Disk scan did not populate native rows")
        let capacityURL=URL(fileURLWithPath:ProcessInfo.processInfo.environment["RETRIEVER_HEALTH_FIXTURES"] ?? "").deletingLastPathComponent().appendingPathComponent("disk-capacity.json")
        model.capacityTestFixture=try JSONSerialization.jsonObject(with:Data(contentsOf:capacityURL)) as? [String:Any]
        try await model.refreshDisk();try require(model.diskEntries.count==6 && model.diskData["fixture_only"] as? Bool==true,"Synthetic capacity dashboard failed")
        try await render(model,output.appendingPathComponent("disk-capacity.png"));try await render(model,output.appendingPathComponent("disk-capacity-narrow.png"),width:820)
        model.capacityTestFixture=nil;try await model.refreshDisk()
        let diskTab=model.activeTabID;let scanID=model.location["scan"] as? String
        model.diskSelection=model.diskEntries.first{$0.name=="budget.xlsx"}?.id;await model.findDiskRelated()
        try require(model.browseRoute=="library" && model.query=="budget" && model.files.contains{$0.name=="budget.xlsx"},"Disk-to-search handoff failed")
        await model.switchTab(diskTab)
        try require(model.browseRoute=="disk" && model.location["scan"] as? String==scanID,"Disk tab lost its snapshot")
        await model.diskMode("largest");try require(model.diskEntries.allSatisfy{!$0.folder},"Largest-files mode includes folders")
        try await render(model,output.appendingPathComponent("disk-space.png"))
        try await render(model,output.appendingPathComponent("disk-space-narrow.png"),width:820)
        model.rememberTabs();await model.restoreTabs()
        try require(model.browseRoute=="disk" && model.location["scan"] as? String==scanID,"Disk session restoration failed")
        model.query="Sequoia budget";await model.submitSearch()
        try require(model.browseRoute=="library" && model.files.first?.name=="budget.xlsx","Search retained the disk route")
        try require(diskBytes(1024)=="1.0 KiB","Binary size units are inconsistent")
        var missingFixtureBlocked=false
        do {_ = try await model.request("health/refresh",body:[:])}catch{missingFixtureBlocked=true}
        try require(missingFixtureBlocked,"Test build allowed a health request without fixtures")
        model.healthTestProvider=try HealthFixtures(url:URL(fileURLWithPath:ProcessInfo.processInfo.environment["RETRIEVER_HEALTH_FIXTURES"] ?? ""))
        var fixtureMonitoringBlocked=false
        do {_ = try await model.request("health/settings",body:["interval_minutes":1])}catch{fixtureMonitoringBlocked=true}
        try require(fixtureMonitoringBlocked,"Fixture provider allowed live monitoring")
        await model.openDriveHealth()
        try require(model.browseRoute=="health" && model.section=="health","Health tab did not open")
        try require(!model.healthAvailable,"Fixture live collection was enabled")
        try require(model.healthDrives.count==4,"Fixture drives not listed")
        try require(model.healthDrive["status"] is String,"Drive health missing explicit status")
        try require((model.healthData["model_calls"] as? NSNumber)?.intValue==0,"Health triggered inference")
        try await render(model,output.appendingPathComponent("drive-health.png"))
        try await render(model,output.appendingPathComponent("drive-health-narrow.png"),width:820)
        await model.selectHealth(String(repeating:"2",count:64));try require(model.healthDrive["status"] as? String=="Caution","HDD caution fixture failed")
        try await render(model,output.appendingPathComponent("drive-health-hdd.png"))
        await model.selectHealth(String(repeating:"3",count:64));try require(model.healthDrive["status"] as? String=="Unknown","Unsupported fixture became healthy")
        await model.selectHealth(String(repeating:"4",count:64));try require(model.healthDrive["present"] as? Bool==false,"Disconnected fixture failed")
        await model.selectHealth(String(repeating:"1",count:64));try require((model.healthDetail["events"] as? [Any])?.count==2,"Alert recovery fixture missing")
        model.location["mode"]=1;let healthDriveID=model.location["drive"] as? String
        try require(!model.healthHistory.isEmpty,"Health history missing")
        model.rememberTabs();await model.restoreTabs()
        try require(model.browseRoute=="health" && model.location["drive"] as? String==healthDriveID && (model.location["mode"] as? NSNumber)?.intValue==1,"Health tab restore failed")
        model.query="Sequoia budget";await model.submitSearch()
        try require(model.browseRoute=="library" && model.files.first?.name=="budget.xlsx","Search retained the health route")
        try require(fileLength(3661)=="1:01:01" && fileLength(65)=="1:05","Video length formatting failed")
        let connection=(model.endpoint,model.token,model.clientMode);var badPairingRejected=false
        do{try await model.changeConnection(client:true,endpoint:model.endpoint,token:"invalid-fixture-token")}catch{badPairingRejected=true}
        try require(badPairingRejected && model.endpoint==connection.0 && model.token==connection.1 && model.clientMode==connection.2,"Failed pairing replaced the active coordinator")
        await model.navigate("files");model.visibleColumns.insert("duration");await model.readLengths()
        try require(model.files.first(where:{$0.name=="sample.mp4"})?.duration != nil,"Video metadata length missing")
        try require(model.lengthSummary.contains("Σ"),"Length sum missing")
        try await render(model,output.appendingPathComponent("video-lengths.png"))
        await model.openAssistant();try await model.refreshAssistant()
        try require(model.section=="assistant" && ((model.assistant["files"] as? NSNumber)?.intValue ?? 0)>0,"Assistant did not show the index")
        model.rememberTabs();await model.restoreTabs();try require(model.section=="assistant","Assistant tab did not restore")
        try await render(model,output.appendingPathComponent("disk-assistant.png"))
        await model.openBurner();try await model.refreshBurner()
        try require(model.section=="burner" && model.browseRoute=="burner","Disk Burner route did not open")
        try require(model.burnerData["physical_writes"] as? Bool==false,"Physical Disk Burner writes enabled")
        let burnerSample=try await model.request("burner/sample",body:[:]);try await model.inspectBurner(burnerSample["path"] as? String ?? "")
        for _ in 0..<200 {try await model.refreshBurner();let job=model.burnerData["current"] as? [String:Any] ?? [:];if !["inspecting","preflight","writing","verifying"].contains(job["state"] as? String ?? ""){break};try await Task.sleep(for:.milliseconds(40))}
        let inspection=model.burnerData["inspection"] as? [String:Any] ?? [:]
        try require(inspection["state"] as? String=="ready","Burner image inspection failed")
        _ = try await model.request("burner/start",body:["inspection_id":inspection["id"] ?? "","target_id":"sandbox-8"])
        for _ in 0..<200 {try await model.refreshBurner();let job=model.burnerData["current"] as? [String:Any] ?? [:];if !["inspecting","preflight","writing","verifying"].contains(job["state"] as? String ?? ""){break};try await Task.sleep(for:.milliseconds(40))}
        let burnerJob=model.burnerData["current"] as? [String:Any] ?? [:];let burnerImage=burnerJob["image"] as? [String:Any] ?? [:]
        try require(burnerJob["state"] as? String=="complete" && (burnerJob["verified_bytes"] as? NSNumber)?.intValue==4*1024*1024,"Burner file rehearsal failed")
        try require(burnerJob["verified_sha256"] as? String==burnerImage["sha256"] as? String,"Burner read-back digest mismatch")
        try await render(model,output.appendingPathComponent("disk-burner.png"));try await render(model,output.appendingPathComponent("disk-burner-narrow.png"),width:820)
        try await renderComponent(BurnerReceiptView(job:burnerJob),output.appendingPathComponent("burner-receipt.png"),width:372,height:400)
        model.rememberTabs();await model.restoreTabs();try require(model.section=="burner","Disk Burner tab did not restore")
        await model.navigate("files");model.query="Sequoia";await model.submitSearch()
        try await renderComponent(SearchDetailPanel(model:model,close:{}),output.appendingPathComponent("expanded-search.png"),width:760,height:390)
        try await renderComponent(UtilityPopover(model:model,kind:"Connections"),output.appendingPathComponent("connections.png"),width:380,height:400)
        try await foundation(model,fixtures,output)
        let report:[String:Any]=["passed":true,"health_fixture_only":true,"live_health_collection":false,"native_model_checks":checks,"synthetic_content_top1":7,"synthetic_content_queries":7,"query_wall_ms":timings,"coordinator_started_and_reused":true,"quick_search_isolated":true,"legacy_state_protected":true,"tabs_restore_and_isolation_verified":true,"typed_properties_and_collections_verified":true,"offscreen_renders":renders,"interactive_desktop_verified":false,"global_hotkey_verified":false,"production_state_modified":false]
        try JSONSerialization.data(withJSONObject:report,options:[.prettyPrinted,.sortedKeys]).write(to:output.appendingPathComponent("native-smoke.json"))
        print(String(data:try JSONSerialization.data(withJSONObject:report,options:.sortedKeys),encoding:.utf8)!)
    }
}
#endif
