import SwiftUI
import AppKit

func fileLength(_ seconds:Double)->String {
    let n=max(0,Int(seconds));return n>=3600 ? String(format:"%d:%02d:%02d",n/3600,(n/60)%60,n%60):String(format:"%d:%02d",n/60,n%60)
}
extension FileItem {
    var duration:Double? {guard let value=(raw["duration_seconds"] as? NSNumber)?.doubleValue,value.isFinite,value>0 else{return nil};return value}
    var length:String {duration.map(fileLength) ?? "—"}
}
@MainActor extension WorkspaceModel {
    func openAssistant() async {await navigateDestination(state:["route":"assistant","section":"assistant","title":"Disk Assistant","query":"","location":[:]])}
    func refreshAssistant() async throws {assistant=try await request("assistant")}
    func browseLocations(_ id:String) async {
        #if RETRIEVER_TESTING
        if healthTestProvider != nil && ProcessInfo.processInfo.environment["RETRIEVER_FIXTURE_SHOW_FILESYSTEM"] != "1" {locations[id]=(devices.first{$0.id==id}?.roots ?? []).map{["path":$0,"name":URL(fileURLWithPath:$0).lastPathComponent,"kind":"fixture"]};return}
        #endif
        await run {self.locations[id]=try await self.request("explorer/locations",body:["device":id])["locations"] as? [[String:Any]] ?? []}
    }
    var lengthSummary:String {let selected=files.filter{selection.contains($0.id)};let source=selected.isEmpty ? files:selected;let measured=source.compactMap(\.duration);return measured.isEmpty || !sumLengths ? "":" · Σ \(fileLength(measured.reduce(0,+))) (\(measured.count) measured · \(selected.isEmpty ? "this page":"selection"))"}
    func readLengths() async {
        let generation=sequence
        for file in files.filter({$0.type=="video" && $0.duration==nil}).prefix(20) {
            guard generation==sequence else{return}
            if let data=try? await request("explorer/duration",body:["device":file.device,"path":file.path]),generation==sequence,let index=files.firstIndex(where:{$0.id==file.id}) {var raw=files[index].raw;raw["duration_seconds"]=data["duration_seconds"];files[index]=FileItem(raw:raw)}
        }
    }
}
struct SearchDetailPanel:View {
    @ObservedObject var model:WorkspaceModel
    var close:()->Void
    @State private var selected:String?
    @State private var text="Select a result to preview it here."
    var body:some View {
        VStack(spacing:12){
            HStack{ForEach([("hybrid","Smart search"),("exact","Words"),("semantic","Meaning")],id:\.0){key,label in Button(label){model.searchMode=key;Task{await model.submitSearch()}}.foregroundStyle(model.searchMode==key ? Color.accentColor:Color.secondary)};Spacer();Button{close()}label:{Image(systemName:"xmark")}.accessibilityLabel("Close search details")}.buttonStyle(.borderless)
            HStack(spacing:12){
                if model.query.isEmpty {VStack(alignment:.leading,spacing:12){Text("Your files, with context").font(.headline);Text("Find a filename, a phrase from a document, or describe what you remember.").foregroundStyle(.secondary);Spacer()}.frame(maxWidth:.infinity)}
                else {List(model.files,selection:$selected){file in VStack(alignment:.leading,spacing:4){Text(file.name).lineLimit(1);Text(file.detail).font(.caption).foregroundStyle(.secondary).lineLimit(1)}.tag(file.id)}.listStyle(.plain).scrollContentBackground(.hidden)}
                Divider();ScrollView{Text(text).font(.system(size:12)).textSelection(.enabled).frame(maxWidth:.infinity,alignment:.leading)}.frame(maxWidth:.infinity)
            }
            HStack{Text(model.message).font(.caption).foregroundStyle(.secondary).lineLimit(1);Spacer();Button("Open"){if let selected {model.selection=[selected];close();Task{await model.open()}}}.disabled(selected==nil)}
        }.padding(16).background(.regularMaterial,in:RoundedRectangle(cornerRadius:12)).overlay(RoundedRectangle(cornerRadius:12).stroke(.quaternary)).shadow(color:.black.opacity(0.22),radius:16,y:8)
        .task(id:selected){guard let file=model.files.first(where:{$0.id==selected}) else{return};text="Loading preview…";do{let result=try await model.request(file.raw["indexed_scope"] as? Bool==false ? "explorer/preview":"workspace/preview",body:["device":file.device,"path":file.path]);if !Task.isCancelled{text=result["text"] as? String ?? file.detail}}catch{if !Task.isCancelled{text=error.localizedDescription}}}
        .onChange(of:model.files.map(\.id)){_,ids in if selected==nil || !ids.contains(selected!){selected=ids.first}}
    }
}
struct DiskAssistantView:View {
    @ObservedObject var model:WorkspaceModel
    @State private var folders=false
    private func count(_ key:String)->Int {(model.assistant[key] as? NSNumber)?.intValue ?? 0}
    var body:some View {
        ScrollView{VStack(alignment:.leading,spacing:20){
            Text("Disk Assistant").font(.system(size:26,weight:.semibold))
            Text("What Retriever has learned, what is running, and what stays on your device.").foregroundStyle(.secondary)
            HStack(spacing:12){metric("Indexed files",count("files"));metric("Searchable passages",count("passages"));metric("Generated descriptions",count("descriptions"))}
            GroupBox("Your retrieval archive"){VStack(spacing:14){ForEach([("Crawl · selected folders",(model.assistant["folders"] as? [Any])?.count ?? 0),("Extract · passages",count("passages")),("Understand · descriptions",count("descriptions")),("Retrieve · vector records",count("vectors"))],id:\.0){label,value in HStack{Text(label).frame(width:200,alignment:.leading);ProgressView(value:Double(value),total:Double(max(1,count("passages"),count("vectors"))));Text(value.formatted()).monospacedDigit().frame(width:70,alignment:.trailing)}}}.padding(12)}
            GroupBox("Model ensemble"){VStack(alignment:.leading,spacing:14){ForEach(Array((model.assistant["models"] as? [[String:Any]] ?? []).enumerated()),id:\.offset){_,role in HStack{VStack(alignment:.leading,spacing:4){Text(role["role"] as? String ?? "Model");Text("\(role["model"] as? String ?? "") · \(role["state"] as? String ?? "Unknown")").font(.caption).foregroundStyle(.secondary)};Spacer();Text(role["verified"] as? Bool==true ? "Verified configuration":"Not verified").font(.caption).foregroundStyle(.secondary)}}
                Text("Coordinator uptime: \(count("uptime_seconds")) s. Model server uptime is not reported by the configured APIs.").font(.caption).foregroundStyle(.secondary)
            }.padding(12)}
            HStack{Button("Crawl folders & schedules"){folders=true}.popover(isPresented:$folders){CrawlFoldersPopover(model:model)};Button("Models & API settings"){Task{await model.navigate("settings")}};Button("Refresh"){Task{await model.run{try await model.refreshAssistant()}}}}
            Text("RAG archive · \(model.assistant["archive_path"] as? String ?? "")\nDerived catalogue metadata refreshes after each crawl. Source text and vectors stay in the private search index.").font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
        }.padding(24).frame(maxWidth:1080,alignment:.leading)}.task{await model.run{try await model.refreshAssistant()}}
    }
    private func metric(_ title:String,_ value:Int)->some View {VStack(alignment:.leading,spacing:8){Text(title.uppercased()).font(.system(size:10)).foregroundStyle(.secondary);Text(value.formatted()).font(.system(size:26,weight:.semibold))}.frame(maxWidth:.infinity,alignment:.leading).padding(14).background(.quaternary.opacity(0.25),in:RoundedRectangle(cornerRadius:10))}
}
struct CrawlFoldersPopover:View {
    @ObservedObject var model:WorkspaceModel
    @Environment(\.dismiss) var dismiss
    var body:some View {VStack(alignment:.leading,spacing:12){Text("AI search · crawl folders").font(.headline);Button("Explorer & search setup"){dismiss();model.crawlSetupPlan=[:];model.showingExplorerSetup=true};ScrollView{VStack(alignment:.leading,spacing:12){ForEach(Array(model.crawlFolders.enumerated()),id:\.offset){_,folder in Text(folder["root"] as? String ?? "").lineLimit(2);Text("\((folder["interval_minutes"] as? NSNumber)?.intValue ?? 0) min · \(folder["state"] as? String ?? "")").font(.caption).foregroundStyle(.secondary)}}}.frame(maxHeight:260);Button("Choose a crawl folder"){dismiss();Task{await model.includeFolder()}};Button("Schedules & analysis settings"){dismiss();Task{await model.navigate("crawl")}}}.padding(16).frame(width:360).task{try? await model.refreshCrawl()}}
}
struct UtilityPopover:View {
    @ObservedObject var model:WorkspaceModel
    let kind:String
    @Environment(\.dismiss) var dismiss
    @State private var detail=""
    @State private var addingDevice=false
    @State private var folderIntegration=false
    @State private var storageDevice=""
    @State private var showingStorage=false
    @State private var connected=Set<String>()
    @State private var discovered:[[String:Any]]=[]
    var body:some View {
        VStack(alignment:.leading,spacing:12) {
            Text(kind).font(.headline)
            if kind=="Desktop app" {desktop}
            else if kind=="Connections" {connections}
            else {activity}
        }.padding(16).frame(width:340).task{try? await model.refresh()}
    }
    private var desktop:some View {
        VStack(alignment:.leading,spacing:12) {
            Text("Retriever for macOS")
            Text("Quick Search is available from the menu bar and Control + Option + Space.").foregroundStyle(.secondary)
            Button("Folder integration"){folderIntegration=true}.popover(isPresented:$folderIntegration){FolderIntegrationPopover(model:model)}
            CoordinatorConnectionForm(model:model)
        }
    }
    private var connections:some View {
        VStack(alignment:.leading,spacing:12) {
            ForEach(model.devices) {device in
                VStack(alignment:.leading,spacing:6){HStack {Text(device.label);Spacer();Button("Check"){Task{await check(device.id)}}};ProgressView(value:connected.contains(device.id) ? 1:0,total:1).tint(.yellow);Button("Browse files"){Task{await model.browseLocations(device.id);if let path=model.locations[device.id]?.first?["path"] as? String{dismiss();await model.browse(device:device.id,path:path)}}};if device.id != model.localDevice {Button("Storage snapshot"){storageDevice=device.id;showingStorage=true}}}
            }
            ForEach(Array(discovered.enumerated()),id:\.offset){_,node in Text("\(node["name"] ?? "Device") · \(node["address"] ?? "") · \(node["online"] as? Bool==true ? "Online":"Offline")").font(.caption)}
            Button("Discover Tailscale devices"){Task{do{discovered=try await model.request("nodes/discover")["nodes"] as? [[String:Any]] ?? [];detail="Pair a saved SSH alias and matching agent. Tailscale presence alone does not grant file access."}catch{detail=error.localizedDescription}}}
            Button("Add remote device"){addingDevice=true}.popover(isPresented:$addingDevice){AddRemoteDevicePopover(model:model)}
            Text(detail).font(.caption).foregroundStyle(.secondary)
            Text("Health collection remains blocked.").font(.caption)
        }.popover(isPresented:$showingStorage){RemoteStoragePopover(model:model,device:storageDevice)}
    }
    private func check(_ id:String) async {
        do {let data=try await model.request("device-probe",body:["device":id]);connected.insert(id);detail="Connected · \(data["platform"] ?? "") · \(data["engine_version"] ?? "")"}
        catch {detail=error.localizedDescription}
    }
    private var activity:some View {
        VStack(alignment:.leading,spacing:12) {
            ScrollView {
                VStack(alignment:.leading,spacing:10) {
                    if model.history.isEmpty {Text("No file operations yet.").foregroundStyle(.secondary)}
                    ForEach(Array(model.history.prefix(12).enumerated()),id:\.offset) {_,entry in
                        Text("\(entry["operation"] as? String ?? "") · \(entry["state"] as? String ?? "")")
                    }
                }
            }.frame(maxHeight:300)
            Button("Full history & undo"){dismiss();Task{await model.navigate("activity")}}
        }
    }
}
struct BatchFilesPopover:View {
    @ObservedObject var model:WorkspaceModel
    let sourceFiles:[FileItem]?
    @State private var operation:String
    @State private var targetDevice:String
    @State private var input:String
    @State private var error=""
    @State private var plan:[String:Any]=[:]
    @State private var working=false
    @State private var progress=""
    @State private var copyingID:String?
    @Environment(\.dismiss) var dismiss
    init(model:WorkspaceModel,operation:String="rename",sourceFiles:[FileItem]?=nil,destination:String="",target:String="") {
        self.model=model;self.sourceFiles=sourceFiles;_operation=State(initialValue:operation)
        _targetDevice=State(initialValue:target.isEmpty ? sourceFiles?.first?.device ?? model.selected?.device ?? "":target)
        _input=State(initialValue:operation=="rename" ? "{name} ({n}){ext}":destination)
    }
    var selected:[FileItem] {sourceFiles ?? model.files.filter{model.selection.contains($0.id)}}
    var remoteTrash:Bool {operation=="trash" && (model.clientMode || selected.first?.device != model.localDevice)}
    var body:some View {
        VStack(alignment:.leading,spacing:12){
            Text("\(selected.count) selected").font(.headline)
            Picker("Action",selection:$operation){Text("Rename").tag("rename");Text("Move").tag("move");Text("Copy").tag("copy");Text("Move to Trash").tag("trash")}.disabled(!plan.isEmpty || working)
                .onChange(of:operation){_,value in input=value=="rename" ? "{name} ({n}){ext}":model.location["path"] as? String ?? ""}
            if operation=="copy" {
                Picker("Destination device",selection:$targetDevice){ForEach(model.devices){device in Text(device.label).tag(device.id)}}.disabled(!plan.isEmpty || working)
                    .onChange(of:targetDevice){_,id in Task{await model.browseLocations(id);if targetDevice==id {input=model.locations[id]?.first?["path"] as? String ?? ""}}}
            }
            if remoteTrash{Text("Remote files move to hidden Retriever Trash beside the original. Restore from Activity → Review undo.").font(.caption).foregroundStyle(.secondary)}
            if operation != "trash" {
                TextField(operation=="rename" ? "Name pattern · {name}, {n}, {ext}":"Destination folder",text:$input).disabled(!plan.isEmpty || working)
                if operation=="copy" || operation=="move" {
                    Button("Choose folder…"){Task{if let chosen=await model.pickPath(title:"Destination folder",initial:input.isEmpty ? nil:input,device:operation=="copy" ? targetDevice:selected.first?.device){input=chosen}}}.disabled(!plan.isEmpty || working)
                }
            }
            ScrollView{Text(plan.isEmpty ? selected.map(\.name).joined(separator:"\n"):(plan["items"] as? [[String:Any]] ?? []).map{"\($0["source"] ?? "")\n→ \($0["destination"] ?? "")"}.joined(separator:"\n\n")).font(.caption).frame(maxWidth:.infinity,alignment:.leading)}.frame(maxHeight:240)
            if !error.isEmpty{Text(error).foregroundStyle(.red).font(.caption)}
            if !progress.isEmpty{Text(progress).font(.caption).foregroundStyle(.secondary)}
            if let id=copyingID {Button("Stop copy"){Task{do{_ = try await model.request("workspace/cancel",body:["id":id]);progress="Stopping after the current block or native file copy…"}catch{self.error=error.localizedDescription}}}}
            Button(operation=="trash" && !remoteTrash ? "Move to Trash":plan.isEmpty ? "Review changes":"Apply changes"){Task{await apply()}}.disabled(working || selected.isEmpty)
        }.padding(16).frame(width:380)
    }
    private func apply() async {working=true;defer{working=false};do{guard Set(selected.map(\.device)).count==1 else{throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"Select files on one source device."])}
        if operation=="trash" && !remoteTrash {guard !model.clientMode,selected.allSatisfy({$0.device==model.localDevice}) else{throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"Use the source device to trash these files."])};let urls=selected.map{URL(fileURLWithPath:$0.path).standardizedFileURL};for url in urls{let path=url.resolvingSymlinksInPath().path;guard path==url.path else{throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"Linked files require review in Finder."])};guard path != "/",path != model.state.path,!model.state.path.hasPrefix(path+"/"),!path.hasPrefix(model.state.path+"/") else{throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"System roots and Retriever state cannot be trashed."])};guard !urls.contains(where:{$0 != url && url.path.hasPrefix($0.path+"/")}) else{throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"Select a folder or its children, not both."])} };for url in urls{try FileManager.default.trashItem(at:url,resultingItemURL:nil)};try await model.loadFiles();dismiss()}
        else if plan.isEmpty {let items=selected.enumerated().map{index,file -> [String:Any] in let url=URL(fileURLWithPath:file.name);let ext=file.kind=="folder" || url.pathExtension.isEmpty ? "":"."+url.pathExtension;let name=input.replacingOccurrences(of:"{name}",with:file.kind=="folder" ? file.name:url.deletingPathExtension().lastPathComponent).replacingOccurrences(of:"{n}",with:String(index+1)).replacingOccurrences(of:"{ext}",with:ext);return operation=="rename" ? ["path":file.path,"name":name]:["path":file.path,"parent":input]};plan=try await model.request("workspace/plan",body:["device":selected[0].device,"target_device":targetDevice.isEmpty ? selected[0].device:targetDevice,"scope":"filesystem","operation":operation,"parent":input,"items":items])}
        else {
            let receipt:[String:Any]
            if operation=="copy" {
                copyingID=plan["id"] as? String;defer{copyingID=nil}
                receipt=try await model.runCopy(plan){value in
                    let done=(value["completed_bytes"] as? NSNumber)?.doubleValue ?? 0,total=(value["total_bytes"] as? NSNumber)?.doubleValue ?? 0
                    progress=String(format:"%.1f / %.1f MiB · %@",done/1048576,total/1048576,value["phase"] as? String ?? "copying")
                }
            }else{receipt=try await model.request("workspace/apply",body:["id":plan["id"] ?? ""])}
            guard (receipt["receipt"] as? [String:Any])?["ok"] as? Bool==true else{throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"Inspect this operation in Activity: \((receipt["receipt"] as? [String:Any])?["error"] ?? "Incomplete operation")"])}
            try await model.loadFiles();model.message=operation=="copy" ? "Copy complete · checksums verified":"Changes applied";dismiss()
        }
    }catch{self.error=error.localizedDescription}}
}
