import SwiftUI
import AppKit
import UniformTypeIdentifiers

@MainActor extension WorkspaceModel {
    func openBurner() async {await navigateDestination(state:["route":"burner","section":"burner","title":"Disk Burner","query":"","location":[:]])}
    func refreshBurner() async throws {
        burnerData=try await request("burner")
        if section=="burner" {message="Disk Burner · "+burnerStage((burnerData["current"] as? [String:Any])?["state"] as? String ?? "")}
    }
    func inspectBurner(_ path:String) async throws {
        _ = try await request("burner/inspect",body:["path":path,"sha256":burnerChecksum.trimmingCharacters(in:.whitespacesAndNewlines)])
        try await refreshBurner()
    }
    func chooseBurnerImage() async {
        guard canUseLocalFiles else {message="Pair this Mac with the shared library before choosing local images.";return}
        guard let path=await pickPath(title:"Choose a disk image",extensions:["img","iso","bin","raw","zip","gz","xz","dmg","vhd","vhdx"]) else{return}
        await run{try await self.inspectBurner(path)}
    }
}

func burnerStage(_ value:String)->String {
    switch value {
    case "inspecting":return "Inspecting image"
    case "preflight":return "Checking source again"
    case "writing":return "Writing file target"
    case "verifying":return "Reading back every byte"
    case "ready":return "Image inspected"
    case "complete":return "Rehearsal verified"
    case "cancelled":return "Cancelled"
    case "interrupted":return "Interrupted"
    case "failed":return "Task failed"
    default:return "Ready when you are"
    }
}
func burnerBytes(_ value:Any?)->String {ByteCountFormatter.string(fromByteCount:(value as? NSNumber)?.int64Value ?? 0,countStyle:.binary)}

struct DiskBurnerView:View {
    @ObservedObject var model:WorkspaceModel
    @State private var target="sandbox-8"
    @State private var checksumPopover=false
    @State private var review=false
    @State private var receipt:String?
    private var job:[String:Any] {model.burnerData["current"] as? [String:Any] ?? [:]}
    private var inspection:[String:Any] {model.burnerData["inspection"] as? [String:Any] ?? [:]}
    private var image:[String:Any] {inspection["image"] as? [String:Any] ?? [:]}
    private var stage:String {job["state"] as? String ?? ""}
    private var running:Bool {["inspecting","preflight","writing","verifying"].contains(stage)}
    private var targets:[[String:Any]] {model.burnerData["targets"] as? [[String:Any]] ?? []}
    private var capacity:Int64 {(targets.first{$0["id"] as? String==target}?["capacity"] as? NSNumber)?.int64Value ?? 0}
    private var bytes:Int64 {(image["bytes"] as? NSNumber)?.int64Value ?? 0}
    private var canStart:Bool {!running && !image.isEmpty && bytes<=capacity && image["raw_copy"] as? Bool==true}
    private var history:[[String:Any]] {Array((model.burnerData["history"] as? [[String:Any]] ?? []).prefix(6))}
    private let yellow=Color(red:0.90,green:0.74,blue:0.30)
    var body:some View {
        ScrollView {
            VStack(alignment:.leading,spacing:18) {
                HStack(alignment:.top) {
                    VStack(alignment:.leading,spacing:5){Text("Disk Burner").font(.system(size:26,weight:.semibold));Text("Inspect an image. Rehearse the write. Verify every byte.").font(.system(size:12)).foregroundStyle(.secondary)}
                    Spacer();Text("FILE SANDBOX").font(.system(size:10,weight:.medium)).foregroundStyle(yellow).padding(.horizontal,10).padding(.vertical,6).background(yellow.opacity(0.15),in:RoundedRectangle(cornerRadius:5))
                }
                Text("Physical disk writing is disabled in this candidate. Rehearsals use a temporary file, which is removed after the test.").font(.system(size:12)).foregroundStyle(.secondary)
                HStack(spacing:22) {ForEach(Array(["01  Inspect","02  Check","03  Write","04  Verify"].enumerated()),id:\.offset){index,label in Text(label).font(.system(size:12)).foregroundStyle(index==phase ? yellow:.secondary)}}
                ViewThatFits(in:.horizontal) {
                    HStack(alignment:.top,spacing:14){sourceCard.frame(minWidth:300);targetCard.frame(minWidth:270)}
                    VStack(spacing:14){sourceCard;targetCard}
                }
                card("Write verification") {
                    VStack(alignment:.leading,spacing:12) {
                        Text(burnerStage(stage)).font(.system(size:15,weight:.medium))
                        ProgressView(value:Double((job["bytes_done"] as? NSNumber)?.int64Value ?? 0),total:max(1,Double((job["bytes_total"] as? NSNumber)?.int64Value ?? 0))).tint(yellow).accessibilityLabel("Disk Burner current phase progress")
                        Text(job.isEmpty ? "No image selected yet.":"\(burnerBytes(job["bytes_done"])) / \(burnerBytes(job["bytes_total"]))"+(running ? " · \(burnerBytes(job["bytes_per_second"]))/s":"")+(stage=="complete" ? " · SHA-256 match · Temporary target removed":"")).font(.system(size:11)).foregroundStyle(.secondary)
                        if let error=job["error"] as? String {Text(error).font(.system(size:12)).foregroundStyle(.orange)}
                        else {ForEach(image["notes"] as? [String] ?? [],id:\.self){Text($0).font(.system(size:11)).foregroundStyle(.secondary)}}
                        HStack(spacing:8) {
                            Button{review=true}label:{Label("Rehearse write & verify",systemImage:"externaldrive.badge.checkmark")}.disabled(!canStart).popover(isPresented:$review,arrowEdge:.bottom){reviewPopover}
                            Button("Cancel"){Task{await model.run{_ = try await model.request("burner/cancel",body:["id":job["id"] ?? ""]);try await model.refreshBurner()}}}.disabled(!running)
                        }
                    }
                }
                card("Recent activity") {
                    VStack(alignment:.leading,spacing:8) {
                        if history.isEmpty {Text("Inspection and verification receipts appear here.").font(.system(size:11)).foregroundStyle(.secondary)}
                        ForEach(history,id:\.burnerID) {item in
                            Button{receipt=item.burnerID}label:{HStack{Image(systemName:item["state"] as? String=="complete" ? "checkmark.circle":"doc.text");Text("\(burnerStage(item["state"] as? String ?? "")) · \((item["image"] as? [String:Any])?["name"] as? String ?? "Image inspection")").lineLimit(1);Spacer();Image(systemName:"chevron.right").foregroundStyle(.tertiary)}.font(.system(size:12))}.buttonStyle(.plain)
                                .popover(isPresented:Binding(get:{receipt==item.burnerID},set:{if !$0{receipt=nil}}),arrowEdge:.bottom){BurnerReceiptView(job:item)}
                        }
                    }
                }
                Text("Raw IMG / BIN / RAW and partitioned ISO images can be rehearsed. Windows installer preparation, compressed-image conversion and physical USB writing are not available yet. A verified copy does not prove bootability.").font(.system(size:10)).foregroundStyle(.secondary)
            }.padding(24).frame(maxWidth:1080,alignment:.leading).frame(maxWidth:.infinity,alignment:.topLeading)
        }.task {
            while !Task.isCancelled && model.section=="burner" {
                do {try await model.refreshBurner();try await Task.sleep(for:running ? .milliseconds(250):.seconds(1))}
                catch is CancellationError {break}
                catch {model.message=error.localizedDescription;break}
            }
        }
    }
    private var phase:Int {switch stage {case "inspecting":return 0;case "preflight":return 1;case "writing":return 2;case "verifying","complete":return 3;default:return -1}}
    private func card<Content:View>(_ title:String,@ViewBuilder content:()->Content)->some View {
        VStack(alignment:.leading,spacing:12){Text(title.uppercased()).font(.system(size:10)).foregroundStyle(.secondary);content()}.padding(14).frame(maxWidth:.infinity,alignment:.leading).background(Color.primary.opacity(0.025),in:RoundedRectangle(cornerRadius:10)).overlay(RoundedRectangle(cornerRadius:10).stroke(Color.primary.opacity(0.12),lineWidth:1))
    }
    private var sourceCard:some View {
        card("01 · Source image") {
            VStack(alignment:.leading,spacing:10) {
                Text(image["name"] as? String ?? (stage=="inspecting" ? "Inspecting selected image…":"Choose an image")).font(.system(size:15)).lineLimit(2)
                Text(image.isEmpty ? "Local image files · SHA-256 inspection":"\(burnerBytes(image["bytes"])) · \(image["format"] as? String ?? "")\n\(image["partition_style"] as? String ?? "") · Bootability not verified").font(.system(size:11)).foregroundStyle(.secondary)
                HStack {
                    Button{Task{await model.chooseBurnerImage()}}label:{Label("Choose image",systemImage:"folder")}.disabled(running)
                    Button("Checksum"){checksumPopover=true}.popover(isPresented:$checksumPopover,arrowEdge:.bottom){VStack(alignment:.leading,spacing:12){Text("Verify source checksum").font(.headline);Text("Paste the publisher’s SHA-256 before choosing an image. A matching hash confirms integrity, not publisher identity.").font(.caption).foregroundStyle(.secondary);TextField("Optional publisher SHA-256",text:$model.burnerChecksum);Button("Done"){checksumPopover=false}}.padding(16).frame(width:320)}
                }
                Button("Try a sample image"){Task{await model.run{model.burnerChecksum="";let sample=try await model.request("burner/sample",body:[:]);try await model.inspectBurner(sample["path"] as? String ?? "")}}}.disabled(running)
            }
        }
    }
    private var targetCard:some View {
        card("02 · Destination") {
            VStack(alignment:.leading,spacing:10) {
                Text("Disposable file target").font(.system(size:15))
                Picker("File target",selection:$target){ForEach(targets,id:\.burnerID){item in Text(item["name"] as? String ?? "").tag(item.burnerID)}}.labelsHidden().disabled(running)
                Text(image.isEmpty ? "Select an image to check its size.":bytes<=capacity ? "\(burnerBytes(bytes)) of \(burnerBytes(capacity)) · Fits":"Image exceeds \(burnerBytes(capacity)) target capacity.").font(.system(size:11)).foregroundStyle(.secondary)
                Text("Simulated capacity · allocated only as bytes are written. No USB or internal drives are opened.").font(.system(size:11)).foregroundStyle(.secondary)
            }
        }
    }
    private var reviewPopover:some View {
        VStack(alignment:.leading,spacing:12){Text("Review file rehearsal").font(.headline);Text("\(image["name"] as? String ?? "")\n\(burnerBytes(bytes)) → \(burnerBytes(capacity)) file target").font(.system(size:12));Text("Retriever will recheck the source, write a new temporary file, read it back and compare SHA-256. The temporary file is removed afterwards.").font(.caption).foregroundStyle(.secondary);Button("Start rehearsal"){let id=inspection["id"] as? String ?? "";let selected=target;review=false;Task{await model.run{_ = try await model.request("burner/start",body:["inspection_id":id,"target_id":selected]);try await model.refreshBurner()}}}.disabled(!canStart)}.padding(16).frame(width:320)
    }
}
extension Dictionary where Key==String,Value==Any {var burnerID:String {self["id"] as? String ?? ""}}
struct BurnerReceiptView:View {
    let job:[String:Any]
    var body:some View {let image=job["image"] as? [String:Any] ?? [:]
        VStack(alignment:.leading,spacing:10){Text("Verification receipt").font(.headline);Text(burnerStage(job["state"] as? String ?? ""));Text("\(image["name"] as? String ?? "")\n\(burnerBytes(image["bytes"]))\nSource SHA-256\n\(image["sha256"] as? String ?? "Not available")\nRead-back SHA-256\n\(job["verified_sha256"] as? String ?? "Not verified")").font(.system(size:11)).textSelection(.enabled);if let error=job["error"] as? String {Text(error).font(.caption)};Text("Bootability: not verified. Physical writes: disabled.").font(.caption).foregroundStyle(.secondary)}.padding(16).frame(width:340)
    }
}
