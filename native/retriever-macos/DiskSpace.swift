import SwiftUI
import AppKit

private struct RetrieverPathEntry:Identifiable,Hashable {
    let name:String,path:String,folder:Bool
    var id:String {path}
}
@MainActor private final class RetrieverPathBrowser:ObservableObject {
    let workspace:WorkspaceModel,extensions:Set<String>?
    let device:String
    @Published var entries:[RetrieverPathEntry]=[]
    @Published var places:[RetrieverPathEntry]=[]
    @Published var address=""
    @Published var current=""
    @Published var parent=""
    @Published var message=""
    @Published var busy=false
    @Published var selection:String?
    @Published var next:Int?
    @Published var history:[String]=[]
    var generation=0,closed=false
    init(_ workspace:WorkspaceModel,_ extensions:[String]?,_ device:String?){self.workspace=workspace;self.device=device ?? workspace.localDevice;self.extensions=extensions.map{Set($0.map{$0.lowercased()})}}
    var chosen:String? {
        guard address.trimmingCharacters(in:.whitespacesAndNewlines)==current else{return nil}
        if let item=entries.first(where:{$0.path==selection}) {return extensions==nil || !item.folder ? item.path:nil}
        return extensions==nil && !current.isEmpty ? current:nil
    }
    func start(_ initial:String?) async {
        do {let data=try await workspace.request("explorer/locations",body:["device":device]);guard !closed else{return}
            places=(data["locations"] as? [[String:Any]] ?? []).compactMap{item in guard let name=item["name"] as? String,let path=item["path"] as? String else{return nil};return RetrieverPathEntry(name:name,path:path,folder:true)}
            await load(initial ?? places.first?.path ?? NSHomeDirectory(),remember:false)
        }catch{message=error.localizedDescription}
    }
    func load(_ path:String,remember:Bool=true,append:Bool=false) async {
        generation+=1;let request=generation;busy=true;message="Loading folder…"
        defer{if request==generation{busy=false}}
        do {let result=try await workspace.request("explorer/browse",body:["device":device,"path":path,"offset":append ? next ?? 0:0,"sort":"name"])
            guard !closed,request==generation else{return}
            if !append {if remember && !current.isEmpty && current != path{history.append(current)};entries=[];selection=nil}
            current=result["path"] as? String ?? path;address=current;parent=result["parent"] as? String ?? ""
            entries += (result["entries"] as? [[String:Any]] ?? []).compactMap{item in
                guard let name=item["name"] as? String,let path=item["path"] as? String else{return nil};let folder=item["kind"] as? String=="folder"
                guard folder || extensions?.contains((name as NSString).pathExtension.lowercased())==true else{return nil}
                return RetrieverPathEntry(name:name,path:path,folder:folder)
            }
            next=result["next_offset"] as? Int;message="\(entries.count) items shown"+(next==nil ? "":" · More items available")
        }catch{message=error.localizedDescription}
    }
}
@MainActor private struct RetrieverPathPickerView:View {
    @ObservedObject var browser:RetrieverPathBrowser
    let finish:(String?)->Void
    var body:some View {
        VStack(alignment:.leading,spacing:10){
            HStack {
                Button {if let previous=browser.history.popLast(){Task{await browser.load(previous,remember:false)}}} label:{Image(systemName:"chevron.left")}.disabled(browser.busy || browser.history.isEmpty).help("Back")
                Button {Task{await browser.load(browser.parent)}} label:{Image(systemName:"arrow.up")}.disabled(browser.busy || browser.parent.isEmpty).help("Parent folder")
                Menu("Home and volumes"){ForEach(browser.places){place in Button(place.name){Task{await browser.load(place.path)}}}}.disabled(browser.busy)
                Spacer()
                if browser.busy{ProgressView().controlSize(.small)}
            }
            TextField("Folder path",text:$browser.address).textFieldStyle(.roundedBorder).onSubmit{guard !browser.busy else{return};Task{await browser.load(browser.address)}}
            List(browser.entries,selection:$browser.selection){entry in
                HStack(spacing:10){Image(systemName:entry.folder ? "folder":"doc").foregroundStyle(entry.folder ? Color.yellow:Color.secondary);Text(entry.name).lineLimit(1);Spacer()}.contentShape(Rectangle()).tag(entry.path)
                    .onTapGesture(count:2){guard entry.folder,!browser.busy else{return};Task{await browser.load(entry.path)}}
            }.listStyle(.inset).frame(minHeight:240)
            HStack{Text(browser.message).font(.caption).foregroundStyle(.secondary);Spacer();if browser.next != nil {Button("More items"){Task{await browser.load(browser.current,remember:false,append:true)}}.disabled(browser.busy)}}
            HStack{Spacer();Button("Cancel"){finish(nil)}.keyboardShortcut(.cancelAction);Button(browser.extensions==nil ? "Choose folder":"Choose file"){finish(browser.chosen)}.keyboardShortcut(.defaultAction).disabled(browser.busy || browser.chosen==nil)}
        }.padding(16).frame(width:560,height:420)
    }
}
@MainActor private final class RetrieverPathWindow:NSObject,NSWindowDelegate {
    let panel=NSPanel(contentRect:NSRect(x:0,y:0,width:592,height:452),styleMask:[.titled,.closable],backing:.buffered,defer:false)
    let browser:RetrieverPathBrowser
    var continuation:CheckedContinuation<String?,Never>?
    init(_ browser:RetrieverPathBrowser){self.browser=browser;super.init();panel.delegate=self;panel.isReleasedWhenClosed=false}
    func finish(_ path:String?){guard let continuation else{return};self.continuation=nil;browser.closed=true;browser.generation+=1;if let parent=panel.sheetParent{parent.endSheet(panel)};panel.orderOut(nil);panel.contentView=nil;continuation.resume(returning:path)}
    func windowShouldClose(_ sender:NSWindow)->Bool{finish(nil);return false}
}
@MainActor extension WorkspaceModel {
    func pickPath(title:String,initial:String?=nil,extensions:[String]?=nil,device:String?=nil) async -> String? {
        let controller=RetrieverPathWindow(RetrieverPathBrowser(self,extensions,device))
        // Keep one browser instance and one continuation for this picker lifetime.
        let picker=controller.browser
        controller.panel.title=title
        controller.panel.contentView=NSHostingView(rootView:RetrieverPathPickerView(browser:picker,finish:{controller.finish($0)}))
        return await withCheckedContinuation {continuation in
            controller.continuation=continuation
            if let parent=NSApp.keyWindow ?? NSApp.mainWindow {parent.beginSheet(controller.panel)}else{controller.panel.center();controller.panel.makeKeyAndOrderFront(nil)}
            Task{await picker.start(initial)}
        }
    }
}

func diskNumber(_ value:Any?)->Int64 {(value as? NSNumber)?.int64Value ?? 0}
func diskBytes(_ value:Int64)->String {
    var size=abs(Double(value));let units=["B","KiB","MiB","GiB","TiB"];var index=0
    while size>=1024 && index<units.count-1 {size/=1024;index+=1}
    return (value<0 ? "−":"")+String(format:index==0 ? "%.0f":"%.1f",size)+" "+units[index]
}
struct DiskEntry:Identifiable {
    let raw:[String:Any]
    var id:Int64 {diskNumber(raw["id"])}
    var name:String {raw["name"] as? String ?? ""}
    var path:String {raw["path"] as? String ?? ""}
    var folder:Bool {raw["kind"] as? String=="folder"}
    var logical:String {diskBytes(diskNumber(raw["logical"]))}
    var allocated:String {diskBytes(diskNumber(raw["allocated"]))+(diskNumber(raw["allocation_unknown"])>0 ? " + ?":"")}
    var change:String {guard let value=raw["delta"] else{return "—"};return (diskNumber(value)>0 ? "+":"")+diskBytes(diskNumber(value))}
}
@MainActor extension WorkspaceModel {
    var diskEntries:[DiskEntry] {(diskData["entries"] as? [[String:Any]] ?? []).map(DiskEntry.init)}
    var diskCurrent:[String:Any] {diskData["current"] as? [String:Any] ?? [:]}
    var diskScan:[String:Any] {diskData["scan"] as? [String:Any] ?? [:]}
    var diskSummary:[String:Any] {diskScan["summary"] as? [String:Any] ?? [:]}
    var diskRunning:Bool {diskScan["state"] as? String=="scanning"}
    var selectedDisk:DiskEntry? {diskEntries.first{$0.id==diskSelection}}
    func openDiskSpace() async {
        let root=browseRoute=="browse" && location["device"] as? String==localDevice ? location["path"] as? String:nil
        await navigateDestination(state:["route":"disk","section":"disk","title":"Disk Capacity","query":"","location":["root":root ?? NSHomeDirectory()]])
    }
    func loadDisk() async throws {
        let captured=sequence;diskData=[:];diskSelection=nil;section="disk"
        if location["scan"]==nil {
            let status=try await request("disk")
            guard captured==sequence,browseRoute=="disk" else{return}
            if let latest=(status["scans"] as? [[String:Any]])?.first {location["scan"]=latest["id"];location["root"]=latest["root"]}
        }
        try await refreshDisk()
    }
    func refreshDisk() async throws {
        guard browseRoute=="disk",let id=location["scan"] as? String else{return}
        let captured=sequence;diskRequest+=1;let revision=diskRequest
        let body:[String:Any]=["id":id,"node":location["node"] ?? 1,"mode":location["mode"] ?? "children","offset":location["offset"] ?? 0]
        let data=try await request("disk/view",body:body)
        guard captured==sequence,revision==diskRequest,browseRoute=="disk" else{return}
        diskData=data;message="Disk Capacity · "+(diskScan["state"] as? String ?? "Ready to scan");if !diskEntries.contains(where:{$0.id==diskSelection}){diskSelection=nil};rememberTabs()
    }
    func startDiskScan() async {
        guard canUseLocalFiles else {message="Pair this Mac's device identity before analyzing its folders.";return}
        await run {
            let captured=self.sequence
            let result=try await self.request("disk/scan",body:["path":self.location["root"] ?? NSHomeDirectory()])
            guard captured==self.sequence,self.browseRoute=="disk" else{return}
            self.location["scan"]=result["id"];self.location["node"]=1;self.location["offset"]=0;try await self.refreshDisk()
        }
    }
    func chooseDiskFolder() async {
        guard canUseLocalFiles else{message="Pair this Mac's device identity before analyzing its folders.";return}
        guard let path=await pickPath(title:"Choose a folder to measure",initial:location["root"] as? String) else{return}
        location["root"]=path;await startDiskScan()
    }
    func cancelDiskScan() async {guard let id=location["scan"] as? String else{return};await run {_ = try await self.request("disk/cancel",body:["id":id]);try await self.refreshDisk()}}
    func diskFolder(_ id:Int64) async {location["node"]=max(1,id);location["offset"]=0;await run {try await self.refreshDisk()}}
    func diskMode(_ mode:String) async {location["mode"]=mode;location["offset"]=0;await run {try await self.refreshDisk()}}
    func diskPage(_ offset:Int64) async {location["offset"]=max(0,offset);await run {try await self.refreshDisk()}}
    func revealDisk() {
        guard canUseLocalFiles,let entry=selectedDisk else{return}
        guard entry.path.hasPrefix("/"),FileManager.default.fileExists(atPath:entry.path) else{message="This item has moved. Scan again to update the snapshot.";return}
        var file=entry.raw;file["device"]=localDevice
        Task {await revealInRetriever(FileItem(raw:file))}
    }
    func findDiskRelated() async {guard let entry=selectedDisk else{return};await newTab(state:["route":"library","section":"search","title":"Search results","query":(entry.name as NSString).deletingPathExtension,"location":[:]])}
}

struct DiskSpaceView:View {
    @ObservedObject var model:WorkspaceModel
    private let yellow=Color(red:0.90,green:0.74,blue:0.30)
    private var totals:[String:Any] {model.diskRunning ? model.diskSummary["totals"] as? [String:Any] ?? [:]:model.diskCurrent}
    private var folderPath:String {model.diskCurrent["path"] as? String ?? model.location["root"] as? String ?? "Choose a folder or drive"}
    private var summary:String {
        if model.diskScan.isEmpty{return "Disk Capacity · Ready to scan"}
        return "\(diskBytes(diskNumber(totals["logical"]))) logical  ·  \(diskBytes(diskNumber(totals["allocated"]))) allocated  ·  \(diskNumber(totals["files"])) files  ·  \(model.diskScan["state"] ?? "")"
    }
    private var note:String {
        if model.diskScan.isEmpty{return "Scans measure local file metadata. Choose a folder or drive to start."}
        let summary=model.diskSummary
        let volume=summary["volume"] as? [String:Any] ?? [:]
        let capacity=volume.isEmpty ? "":" Volume: \(diskBytes(diskNumber(volume["available"]))) available of \(diskBytes(diskNumber(volume["total"])))."
        let excluded=diskNumber(summary["links_skipped"])+diskNumber(summary["mounts_skipped"])+diskNumber(summary["private_state_skipped"])
        return "\(String(format:"%.1f",Double(diskNumber(summary["elapsed_ms"]))/1000))s · \(diskNumber(summary["errors"])) read errors · \(excluded) links / mounts / private folders excluded · \(diskNumber(totals["allocation_unknown"])) unknown allocations. "+(model.location["mode"] as? String=="older" ? "Age does not mean unused. ":"")+(model.diskData["previous_scan"] is String ? "":"Scan again for size changes. ")+"Allocated size is not a reclaimable-space estimate. "+(model.diskScan["error"] as? String ?? "")+capacity
    }
    var body:some View {
        VStack(spacing:0) {
            actions
            Text((model.diskData["fixture_only"] as? Bool==true ? "SYNTHETIC PREVIEW · ":"")+folderPath).font(.system(size:11)).lineLimit(1).truncationMode(.middle).frame(maxWidth:.infinity,alignment:.leading).help(folderPath).padding(.vertical,8)
            CapacityOverview(model:model)
            treemap.frame(height:152)
            Text(types).font(.system(size:11)).foregroundStyle(.secondary).lineLimit(1).frame(maxWidth:.infinity,alignment:.leading).padding(.vertical,7)
            modes
            headings
            List(selection:$model.diskSelection) {
                ForEach(model.diskEntries) {entry in
                    HStack(spacing:8) {
                        Image(systemName:entry.folder ? "folder.fill":"doc").foregroundStyle(yellow).frame(width:22)
                        Text(entry.name).lineLimit(1).frame(maxWidth:.infinity,alignment:.leading)
                        Text(entry.logical).frame(width:90,alignment:.trailing)
                        Text(entry.allocated).foregroundStyle(.secondary).frame(width:100,alignment:.trailing)
                        Text(entry.change).foregroundStyle(.secondary).frame(width:90,alignment:.trailing)
                    }.font(.system(size:12)).frame(height:31).tag(entry.id).help(entry.path+"\n"+(entry.raw["warning"] as? String ?? "")).listRowSeparator(.hidden).listRowInsets(EdgeInsets(top:0,leading:4,bottom:0,trailing:4))
                }
            }.listStyle(.plain).scrollContentBackground(.hidden).environment(\.defaultMinListRowHeight,31)
                .onTapGesture(count:2){if let entry=model.selectedDisk,entry.folder {Task {await model.diskFolder(entry.id)}}}
                .onKeyPress(.return){if let entry=model.selectedDisk,entry.folder {Task {await model.diskFolder(entry.id)}};return .handled}
            footer
        }.padding(.horizontal,12)
        .task(id:model.activeTabID) {
            while !Task.isCancelled {
                do {try await Task.sleep(for:.seconds(1));guard model.browseRoute=="disk" else{return};if model.diskRunning {try await model.refreshDisk()}}
                catch is CancellationError {return}
                catch {model.message=error.localizedDescription}
            }
        }
    }
    private var actions:some View {
        HStack(spacing:12) {
            Button {Task {await model.chooseDiskFolder()}} label:{Label("Choose folder",systemImage:"folder.badge.plus")}.disabled(!model.canUseLocalFiles)
            Button {Task {await model.startDiskScan()}} label:{Label("Scan / rescan",systemImage:"externaldrive.badge.magnifyingglass")}.disabled(model.diskRunning || !model.canUseLocalFiles)
            Button {Task {await model.cancelDiskScan()}} label:{Label("Cancel scan",systemImage:"xmark")}.disabled(!model.diskRunning)
            Button {Task {await model.diskFolder(diskNumber(model.diskCurrent["parent"]))}} label:{Image(systemName:"arrow.up")}.disabled(diskNumber(model.diskCurrent["parent"])==0).help("Parent folder").accessibilityLabel("Parent folder")
            Button {Task {await model.diskFolder(1)}} label:{Image(systemName:"house")}.help("Scan root").accessibilityLabel("Scan root")
            Spacer()
        }.buttonStyle(.borderless).frame(height:38)
    }
    private var modes:some View {
        HStack {
            Picker("View",selection:Binding(get:{model.location["mode"] as? String ?? "children"},set:{value in Task {await model.diskMode(value)}})) {
                Text("Folders & files").tag("children");Text("Largest files").tag("largest");Text("Not modified in a year").tag("older");Text("Size changes").tag("growth")
            }.labelsHidden().frame(width:200)
            Spacer()
            Button("Open in Retriever"){model.revealDisk()}.disabled(model.selectedDisk==nil || !model.canUseLocalFiles)
            Button {Task {await model.findDiskRelated()}} label:{Label("Find related",systemImage:"magnifyingglass")}.disabled(model.selectedDisk==nil)
        }.buttonStyle(.borderless).frame(height:34)
    }
    private var headings:some View {
        HStack(spacing:8) {Text("Name").frame(maxWidth:.infinity,alignment:.leading);Text("Logical size").frame(width:90,alignment:.trailing);Text("Allocated").frame(width:100,alignment:.trailing);Text("Change").frame(width:90,alignment:.trailing)}.font(.system(size:11)).padding(.horizontal,4).frame(height:28)
    }
    private var types:String {(model.diskData["types"] as? [[String:Any]] ?? []).prefix(5).map{let ext=$0["extension"] as? String ?? "";return (ext.isEmpty ? "No extension":"."+ext)+" "+diskBytes(diskNumber($0["logical"]))}.joined(separator:"   ·   ")}
    private var footer:some View {
        VStack(alignment:.leading,spacing:4) {
            HStack {Text("\(diskNumber(model.diskData["total"])) items · Largest first");Spacer();Button("Previous page"){Task {await model.diskPage(diskNumber(model.location["offset"])-200)}}.disabled(diskNumber(model.location["offset"])==0);Button("Next page"){Task {await model.diskPage(diskNumber(model.diskData["next_offset"]))}}.disabled(!(model.diskData["next_offset"] is NSNumber))}.buttonStyle(.borderless)
            Text(note).font(.system(size:10)).foregroundStyle(.secondary).lineLimit(3).frame(maxWidth:.infinity,alignment:.leading)
        }.font(.system(size:11)).padding(.vertical,6)
    }
    private var treemap:some View {
        let tiles=(model.diskData["treemap"] as? [[String:Any]] ?? []).map(DiskEntry.init)
        return GeometryReader {geometry in
            ZStack(alignment:.topLeading) {
                if tiles.isEmpty {Text(model.diskRunning ? "Measuring folders…":"Your space map will appear here").foregroundStyle(.secondary).padding(16)}
                ForEach(Array(tiles.enumerated()),id:\.element.id) {index,entry in
                    DiskSpaceTile(entry:entry,index:index,size:geometry.size) {
                        if entry.folder {Task {await model.diskFolder(entry.id)}}else{model.diskSelection=entry.id}
                    }
                }
            }
        }
    }
}

private struct DiskSpaceTile:View {
    let entry:DiskEntry
    let index:Int
    let size:CGSize
    let action:()->Void
    private var rect:[String:Double] {entry.raw["rect"] as? [String:Double] ?? [:]}
    private var color:Color {let colors=[Color(red:0.42,green:0.35,blue:0.15),Color(red:0.34,green:0.30,blue:0.19),Color(red:0.24,green:0.29,blue:0.24),Color(red:0.27,green:0.26,blue:0.28),Color(red:0.32,green:0.32,blue:0.23)];return colors[index%colors.count]}
    var body:some View {
        let width=max(0,(rect["width"] ?? 0)*size.width-2)
        let height=max(0,(rect["height"] ?? 0)*size.height-2)
        let x=(rect["x"] ?? 0)*size.width
        let y=(rect["y"] ?? 0)*size.height
        return Button(action:action) {
            Text(entry.name+"\n"+entry.logical).font(.system(size:11)).foregroundStyle(.white).lineLimit(3).opacity(width<65 || height<30 ? 0:1).padding(7).frame(maxWidth:.infinity,maxHeight:.infinity,alignment:.topLeading)
        }.buttonStyle(.plain).frame(width:width,height:height).background(color,in:RoundedRectangle(cornerRadius:3)).clipped().offset(x:x,y:y).disabled(entry.id==0).help(entry.name+" · "+entry.logical).accessibilityLabel(entry.name+", "+entry.logical)
    }
}
