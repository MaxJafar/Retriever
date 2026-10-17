import SwiftUI

struct CoordinatorConnectionForm:View {
    @ObservedObject var model:WorkspaceModel
    @State private var client=false
    @State private var endpoint=""
    @State private var token=""
    @State private var error=""
    @State private var connecting=false
    var body:some View {VStack(alignment:.leading,spacing:10){
        Toggle("Connect to another coordinator",isOn:$client)
        if client {TextField("Coordinator URL",text:$endpoint);SecureField("Pairing token",text:$token)}
        if !error.isEmpty {Text(error).font(.caption).foregroundStyle(.red)}
        Button("Connect"){Task{connecting=true;defer{connecting=false};do{try await model.changeConnection(client:client,endpoint:endpoint,token:token);error=""}catch{self.error=error.localizedDescription}}}.disabled(connecting)
    }.onAppear{client=model.clientMode;endpoint=model.endpoint;token=model.clientMode ? model.token:""}}
}

struct AddRemoteDevicePopover:View {
    @ObservedObject var model:WorkspaceModel
    @Environment(\.dismiss) var dismiss
    @State private var label=""
    @State private var alias=""
    @State private var platform="macos"
    @State private var agent="retriever-agent"
    @State private var feedback=""
    @State private var saving=false
    var body:some View {VStack(alignment:.leading,spacing:12){
        Text("Add remote device").font(.headline)
        Text("Install the matching headless package on the device, then connect through an existing verified SSH alias. The agent runs on demand, without a background service or model server.").font(.caption).foregroundStyle(.secondary)
        TextField("Device name",text:$label);TextField("Saved SSH alias",text:$alias)
        Picker("Operating system",selection:$platform){Text("macOS").tag("macos");Text("Windows").tag("windows");Text("Linux").tag("linux")}
        TextField("Agent executable on that device",text:$agent)
        Text("The headless installer prints the absolute agent path. Use forward slashes on Windows. No passwords are stored here.").font(.caption).foregroundStyle(.secondary)
        if !feedback.isEmpty {Text(feedback).font(.caption).foregroundStyle(.red)}
        Button("Add device"){Task{await save()}}.disabled(saving || label.trimmingCharacters(in:.whitespaces).isEmpty)
    }.padding(16).frame(width:360)}
    private func save() async {saving=true;defer{saving=false};do{var config=model.configuration;var devices=config["devices"] as? [[String:Any]] ?? [];devices.append(["id":"remote-"+UUID().uuidString.lowercased().prefix(12),"label":label.trimmingCharacters(in:.whitespaces),"transport":"ssh","platform":platform,"enabled":true,"ssh":alias.trimmingCharacters(in:.whitespaces),"agent":agent.trimmingCharacters(in:.whitespaces),"roots":[String](),"exclude":[String](),"analysis_roots":[String]()]);config["devices"]=devices;_ = try await model.request("config",body:config,revision:model.revision);try await model.refresh();model.message="Device added. Check its connection; AI crawling is off.";dismiss()}catch{feedback=error.localizedDescription}}
}
struct RemoteStoragePopover:View {
    @ObservedObject var model:WorkspaceModel
    let device:String
    @State private var path=""
    @State private var working=false
    @State private var result:[String:Any]=[:]
    @State private var error=""
    private var largest:[[String:Any]] {result["largest"] as? [[String:Any]] ?? []}
    private func bytes(_ value:Any?)->String {ByteCountFormatter.string(fromByteCount:(value as? NSNumber)?.int64Value ?? 0,countStyle:.binary)}
    var body:some View {VStack(alignment:.leading,spacing:12){
        Text("Remote storage").font(.headline)
        TextField("Absolute folder path on the device",text:$path)
        Text("Bounded directory metadata over SSH. No models or drive health probes. Large folders may return partial totals.").font(.caption).foregroundStyle(.secondary)
        if working {ProgressView().controlSize(.small)}
        if !result.isEmpty {Text(bytes(result["logical_bytes"])).font(.system(size:25,weight:.semibold));Text("\((result["files"] as? NSNumber)?.intValue ?? 0) files · \((result["duration_ms"] as? NSNumber)?.intValue ?? 0) ms · \(result["complete"] as? Bool==true ? "Complete snapshot":"Partial snapshot")").font(.caption)
            Text("\((result["skipped"] as? NSNumber)?.intValue ?? 0) entries skipped · logical bytes; hard links counted per path").font(.caption).foregroundStyle(.secondary)
            ScrollView{VStack(alignment:.leading,spacing:10){ForEach(Array(largest.prefix(8).enumerated()),id:\.offset){_,file in VStack(alignment:.leading,spacing:4){Text("\(file["name"] as? String ?? "") · \(bytes(file["logical"]))").font(.caption).lineLimit(1);ProgressView(value:(file["logical"] as? NSNumber)?.doubleValue ?? 0,total:max(1,(largest.first?["logical"] as? NSNumber)?.doubleValue ?? 1)).tint(.yellow)}}}}.frame(maxHeight:260)
        }
        if !error.isEmpty {Text(error).font(.caption).foregroundStyle(.red)}
        Text("Drive health · Unavailable. Live collection remains blocked.").font(.caption).foregroundStyle(.secondary)
        Button("Analyze folder"){Task{await scan()}}.disabled(working || path.isEmpty)
    }.padding(16).frame(width:400)}
    private func scan() async {working=true;result=[:];error="";defer{working=false};do{result=try await model.request("devices/storage",body:["device":device,"path":path])}catch{self.error=error.localizedDescription}}
}
