import SwiftUI
import AppKit

func healthValue(_ value:Any?,_ suffix:String="")->String {guard let value,!(value is NSNull) else{return "—"};return String(describing:value)+suffix}
func healthDate(_ value:Any?)->String {guard let n=value as? NSNumber else{return "—"};return Date(timeIntervalSince1970:n.doubleValue).formatted(date:.abbreviated,time:.shortened)}
func healthBytes(_ value:Any?)->String {guard let value,let number=Int64(String(describing:value)) else{return "—"};return diskBytes(number)}

@MainActor extension WorkspaceModel {
    var healthDrives:[[String:Any]] {healthData["drives"] as? [[String:Any]] ?? []}
    var healthDrive:[String:Any] {healthDetail["drive"] as? [String:Any] ?? [:]}
    var healthSettings:[String:Any] {healthData["settings"] as? [String:Any] ?? [:]}
    var healthHistory:[[String:Any]] {healthDetail["history"] as? [[String:Any]] ?? []}
    var healthAvailable:Bool {healthData["probe_available"] as? Bool ?? false}
    var healthRefreshing:Bool {healthData["refreshing"] as? Bool ?? false}
    func openDriveHealth() async {await navigateDestination(state:["route":"health","section":"health","title":"Disk Health","query":"","location":[:]])}
    func loadHealth() async throws {
        section="health";healthDetail=[:];healthData=[:];let captured=sequence
        try await refreshHealth()
        guard captured==sequence,browseRoute=="health" else{return}
        if healthAvailable && healthDrives.isEmpty && !healthRefreshing {_ = try await request("health/refresh",body:[:]);try await refreshHealth()}
    }
    func refreshHealth() async throws {
        guard browseRoute=="health" else{return};let captured=sequence;healthRequest+=1;let revision=healthRequest;let selected=location["drive"] as? String
        let data=try await request("health")
        guard captured==sequence,revision==healthRequest,browseRoute=="health" else{return}
        let drives=data["drives"] as? [[String:Any]] ?? [];let drive=drives.first{$0["id"] as? String==selected} ?? drives.first{$0["present"] as? Bool==true} ?? drives.first
        let id=drive?["id"] as? String;var detail:[String:Any]=[:]
        if let id {detail=try await request("health/detail",body:["id":id])}
        guard captured==sequence,revision==healthRequest,browseRoute=="health",selected==location["drive"] as? String else{return}
        healthData=data;healthDetail=detail;if let id {location["drive"]=id};message="Disk Health · "+(healthRefreshing ? "Checking…":"Saved "+healthDate(drive?["sampled_at"]))+" · "+(clientMode ? "Coordinator computer":"This computer")+" · No AI calls";rememberTabs()
    }
    func checkHealth() async {await run {if self.healthAvailable {_ = try await self.request("health/refresh",body:[:])};try await self.refreshHealth()}}
    func selectHealth(_ id:String) async {location["drive"]=id;await run{try await self.refreshHealth()}}
    func setHealthSettings(interval:Int?=nil,temperature:Int?=nil,life:Int?=nil) async {
        var settings=healthSettings;if !healthAvailable{settings["interval_minutes"]=0};if let interval{settings["interval_minutes"]=interval};if let temperature{settings["temperature_c"]=temperature};if let life{settings["remaining_life_percent"]=life}
        await run{_ = try await self.request("health/settings",body:settings);try await self.refreshHealth()}
    }
    func copyHealthReport(){guard JSONSerialization.isValidJSONObject(healthDetail),let data=try? JSONSerialization.data(withJSONObject:healthDetail,options:[.prettyPrinted,.sortedKeys]),let text=String(data:data,encoding:.utf8) else{return};NSPasteboard.general.clearContents();NSPasteboard.general.setString(text,forType:.string);message="Disk Health report copied. Includes drive identity, readings, history and alerts."}
}

struct DriveHealthView:View {
    @ObservedObject var model:WorkspaceModel
    @State private var thresholds=false
    @State private var temperature=70
    @State private var life=10
    private var drive:[String:Any] {model.healthDrive}
    private var mode:Int {(model.location["mode"] as? NSNumber)?.intValue ?? 0}
    private var status:String {drive["status"] as? String ?? "Unknown"}
    private var color:Color {status=="Bad" ? Color(red:0.93,green:0.5,blue:0.48):status=="Caution" ? Color(red:0.9,green:0.74,blue:0.3):status=="Good" ? Color(red:0.54,green:0.72,blue:0.6):.secondary}
    private var current:[String:Any] {model.healthDrives.first{$0["id"] as? String==drive["id"] as? String} ?? [:]}
    var body:some View {
        VStack(alignment:.leading,spacing:8) {
            actions
            Picker("Drive",selection:Binding(get:{model.location["drive"] as? String ?? ""},set:{id in Task{await model.selectHealth(id)}})) {
                if model.healthDrives.isEmpty {Text("No drive readings yet").tag("")}
                ForEach(model.healthDrives,id:\.healthIdentity){d in Text(driveLabel(d)).tag(d["id"] as? String ?? "")}
            }.labelsHidden().frame(maxWidth:.infinity).controlSize(.large)
            Text(model.healthData["fixture_only"] as? Bool==true ? "SYNTHETIC PREVIEW · No data from your drives":model.healthAvailable ? "":"Live checks disabled · Saved reports only").font(.system(size:12)).foregroundStyle(Color(red:0.9,green:0.74,blue:0.3))
            Text(drive["model"] as? String ?? "Disk Health").font(.system(size:20,weight:.semibold)).lineLimit(1)
            HealthOverview(model:model)
            Text(info).font(.system(size:11)).foregroundStyle(.secondary).textSelection(.enabled)
            HStack {
                Picker("View",selection:Binding(get:{mode},set:{v in model.location["mode"]=v;model.rememberTabs()})) {Text("S.M.A.R.T. attributes").tag(0);Text("Temperature history").tag(1);Text("Health alerts").tag(2)}.labelsHidden().frame(width:190)
                Text(mode==0 ? "Current / Worst / Threshold / Raw":mode==1 ? "Latest 180 samples · °C":"Health transitions").font(.system(size:11)).foregroundStyle(.secondary)
                Spacer()
            }
            if mode==1 {HealthTrend(samples:model.healthHistory).frame(height:100)}
            ScrollView {
                LazyVStack(spacing:0) {
                    if mode==0 {ForEach(Array((drive["attributes"] as? [[String:Any]] ?? []).enumerated()),id:\.offset){_,a in attributeRow(a)}}
                    else if mode==1 {ForEach(Array(model.healthHistory.reversed().enumerated()),id:\.offset){_,h in HStack{Text(healthDate(h["time"]));Spacer();Text(healthValue(h["temperature_c"]," °C")+" · "+healthValue(h["remaining_percent"],"%"))}.padding(.vertical,7)}}
                    else {ForEach(Array((model.healthDetail["events"] as? [[String:Any]] ?? []).enumerated()),id:\.offset){_,e in VStack(alignment:.leading,spacing:4){Text(healthValue(e["status"])+" · "+healthDate(e["time"])).fontWeight(.medium);Text((e["reasons"] as? [String] ?? []).joined(separator:" ")).foregroundStyle(.secondary)}.frame(maxWidth:.infinity,alignment:.leading).padding(.vertical,7)}}
                }.font(.system(size:11)).textSelection(.enabled)
            }.frame(maxWidth:.infinity,maxHeight:.infinity)
            Text(note).font(.system(size:11)).foregroundStyle(.secondary).lineLimit(5).frame(maxWidth:.infinity,alignment:.leading)
        }.padding(.horizontal,14).padding(.bottom,8)
        .task(id:model.activeTabID){while !Task.isCancelled && model.browseRoute=="health" {do{try await Task.sleep(for:.seconds(model.healthRefreshing ? 2:15));if model.browseRoute=="health"{try await model.refreshHealth()}}catch{if !Task.isCancelled{model.message=error.localizedDescription};break}}}
    }
    private func driveLabel(_ d:[String:Any])->String {
        let parts=[healthValue(d["model"]),healthValue(d["bus"]),healthValue(d["status"])]
        return parts.joined(separator:" · ")+(d["present"] as? Bool==false ? " · Disconnected":"")
    }
    private var actions:some View {
        HStack(spacing:10){
            Button{Task{await model.checkHealth()}}label:{Label("Refresh health",systemImage:"arrow.clockwise")}.disabled(!model.healthAvailable || model.healthRefreshing)
            Text("Monitor").font(.system(size:11))
            Picker("Monitor",selection:Binding(get:{(model.healthSettings["interval_minutes"] as? NSNumber)?.intValue ?? 0},set:{v in Task{await model.setHealthSettings(interval:v)}})){Text("Manual").tag(0);Text("Every minute").tag(1);Text("Every 5 minutes").tag(5);Text("Every 15 minutes").tag(15);Text("Every 30 minutes").tag(30);Text("Every hour").tag(60)}.labelsHidden().frame(width:150).disabled(!model.healthAvailable)
            Button{temperature=(model.healthSettings["temperature_c"] as? NSNumber)?.intValue ?? 70;life=(model.healthSettings["remaining_life_percent"] as? NSNumber)?.intValue ?? 10;thresholds=true}label:{Image(systemName:"slider.horizontal.3")}.help("Alert thresholds").accessibilityLabel("Alert thresholds").popover(isPresented:$thresholds){HealthThresholdPopover(model:model)}
            Button{model.copyHealthReport()}label:{Image(systemName:"doc.on.doc")}.disabled(drive.isEmpty).help("Copy report").accessibilityLabel("Copy health report")
            Spacer()
        }.buttonStyle(.borderless).frame(height:38)
    }
    private var summary:String {status+"  ·  "+healthValue(drive["temperature_c"]," °C")+"  ·  "+healthValue(drive["remaining_percent"],"% rated endurance remaining")+(current["present"] as? Bool==false ? "  ·  Disconnected":current["stale"] as? Bool==true ? "  ·  Saved reading":"")}
    private var info:String {guard !drive.isEmpty else{return ""};return healthValue(drive["bus"])+" · "+healthBytes(drive["capacity_bytes"])+" · Firmware "+healthValue(drive["firmware"])+"\nPower on: "+healthValue(drive["power_on_hours"]," h")+" · Power cycles: "+healthValue(drive["power_cycles"])+" · Host reads: "+healthBytes(drive["host_reads_bytes"])+" · Host writes: "+healthBytes(drive["host_writes_bytes"])+"\n"+healthValue(drive["source"])+" · Saved "+healthDate(drive["sampled_at"])}
    private var note:String {([model.healthData["error"] as? String ?? ""]+(drive["reasons"] as? [String] ?? [])+(drive["limitations"] as? [String] ?? [])).filter{!$0.isEmpty}.joined(separator:"\n")}
    private func attributeRow(_ a:[String:Any])->some View {
        HStack(spacing:8){Text(healthValue(a["id"])).foregroundStyle(.secondary).lineLimit(1).frame(width:34,alignment:.leading);Text(healthValue(a["name"])).lineLimit(1).frame(maxWidth:.infinity,alignment:.leading).help(healthValue(a["name"]));Text(healthValue(a["current"])).frame(width:48,alignment:.trailing);Text(healthValue(a["worst"])).frame(width:48,alignment:.trailing);Text(healthValue(a["threshold"])).frame(width:60,alignment:.trailing);Text(healthValue(a["raw"])+" "+healthValue(a["unit"])).lineLimit(1).frame(width:160,alignment:.trailing).help(healthValue(a["raw"]))}.padding(.vertical,7).overlay(alignment:.bottom){Divider().opacity(0.3)}
    }
}
private extension Dictionary where Key==String,Value==Any {var healthIdentity:String{self["id"] as? String ?? ""}}
private struct HealthTrend:View {
    let samples:[[String:Any]]
    var body:some View {
        let valid=samples.compactMap{($0["temperature_c"] as? NSNumber)?.doubleValue}
        return GeometryReader{geometry in
            if valid.count<2 {Text("Two measured samples are needed for a trend. Monitoring records readings while Retriever is running.").font(.system(size:11)).foregroundStyle(.secondary)}
            else {let low=(valid.min() ?? 0)-2;let high=(valid.max() ?? 1)+2;let first=(samples.first?["time"] as? NSNumber)?.doubleValue ?? 0;let last=(samples.last?["time"] as? NSNumber)?.doubleValue ?? 1
                Path{path in var connected=false;for sample in samples{guard let temp=(sample["temperature_c"] as? NSNumber)?.doubleValue,let time=(sample["time"] as? NSNumber)?.doubleValue else{connected=false;continue};let point=CGPoint(x:4+(time-first)/max(1,last-first)*(geometry.size.width-8),y:90-(temp-low)/(high-low)*80);if connected{path.addLine(to:point)}else{path.move(to:point)};connected=true}}.stroke(Color(red:0.9,green:0.74,blue:0.3),lineWidth:2)
            }
        }.accessibilityLabel("Temperature trend; individual readings follow below")
    }
}
