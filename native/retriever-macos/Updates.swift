import SwiftUI
import AppKit
import Foundation

@MainActor final class AppUpdates: ObservableObject {
    @Published var busy=false
    @Published var downloading=false
    @Published var message="Check for new Retriever releases."
    @Published var available:[String:Any]?
    @Published var automatic=true {didSet{save()}}
    @Published var preview=true {didSet{save()}}
    @Published var skipped=""
    @Published var laterUntil=0.0
    private let folder:URL
    private let repository:String
    private let helper:URL
    private var receipt:String?
    private var downloadVersion=""
    private var process:Process?
    private var input:Pipe?
    private var clock:Task<Void,Never>?
    private var ready=false
    let current:String
    var notify:Bool {guard let version=available?["version"] as? String else{return false};return version != skipped && laterUntil<=Date().timeIntervalSince1970}
    var downloaded:Bool {receipt != nil && downloadVersion == available?["version"] as? String}
    var releases:URL? {URL(string:"https://github.com/"+repository+"/releases")}
    init(state:URL) {
        folder=state.appendingPathComponent("updates",isDirectory:true)
        helper=Bundle.main.bundleURL.appendingPathComponent("Contents/Resources/engine/retriever-updater")
        current=Bundle.main.object(forInfoDictionaryKey:"RetrieverEngineVersion") as? String ?? ""
        let config=Bundle.main.url(forResource:"repository",withExtension:"json").flatMap{try? Data(contentsOf:$0)}.flatMap{try? JSONSerialization.jsonObject(with:$0)} as? [String:Any]
        repository=config?["repository"] as? String ?? ""
        let saved=(try? Data(contentsOf:folder.appendingPathComponent("preferences.json"))).flatMap{try? JSONSerialization.jsonObject(with:$0)} as? [String:Any] ?? [:]
        automatic=saved["automatic"] as? Bool ?? true;preview=saved["preview"] as? Bool ?? current.contains("alpha")
        skipped=saved["skipped"] as? String ?? "";laterUntil=saved["later_until"] as? Double ?? 0
        receipt=saved["receipt"] as? String;downloadVersion=saved["download_version"] as? String ?? "";ready=true
    }
    private func save(){guard ready else{return};do{try FileManager.default.createDirectory(at:folder,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700]);var value:[String:Any]=["automatic":automatic,"preview":preview,"skipped":skipped,"later_until":laterUntil,"download_version":downloadVersion];if let receipt{value["receipt"]=receipt};try JSONSerialization.data(withJSONObject:value).write(to:folder.appendingPathComponent("preferences.json"),options:.atomic)}catch{message="Update preferences could not be saved: \(error.localizedDescription)"}}
    func later(){laterUntil=Date().addingTimeInterval(86400).timeIntervalSince1970;save()}
    func skip(){skipped=available?["version"] as? String ?? "";save()}
    func start(){guard clock==nil,!NativeProfile.isolated else{return};clock=Task{while !Task.isCancelled{await check(manual:false);do{try await Task.sleep(for:.seconds(1800))}catch{return}}}}
    func cancel(){guard downloading else{return};try? input?.fileHandleForWriting.write(contentsOf:Data("CANCEL\n".utf8))}
    private func run(_ arguments:[String]) async throws -> [String:Any] {
        guard repository.range(of:"^[A-Za-z0-9][A-Za-z0-9_.-]{0,99}/[A-Za-z0-9][A-Za-z0-9_.-]{0,99}$",options:.regularExpression) != nil,FileManager.default.isExecutableFile(atPath:helper.path) else {throw NSError(domain:"RetrieverUpdater",code:1,userInfo:[NSLocalizedDescriptionKey:"This build does not include a configured update helper."])}
        let task=Process();let output=Pipe();let errors=Pipe();let stdin=Pipe();task.executableURL=helper;task.arguments=["--repository",repository]+arguments;task.standardOutput=output;task.standardError=errors;task.standardInput=stdin
        try task.run();process=task;input=stdin;defer{process=nil;input=nil}
        let errorData=Task.detached{errors.fileHandleForReading.readDataToEndOfFile()}
        var result:[String:Any]?
        for try await line in output.fileHandleForReading.bytes.lines {
            guard let value=try JSONSerialization.jsonObject(with:Data(line.utf8)) as? [String:Any] else{continue}
            if value["event"] as? String == "progress",let received=value["received"] as? Double,let total=value["total"] as? Double,total>0 {message="Downloading update В· \(Int(received*100/total))%"}else{result=value}
        }
        while task.isRunning{try await Task.sleep(for:.milliseconds(50))}
        let data=await errorData.value
        if task.terminationStatus != 0 {let error=(try? JSONSerialization.jsonObject(with:data)) as? [String:Any];throw NSError(domain:"RetrieverUpdater",code:2,userInfo:[NSLocalizedDescriptionKey:error?["error"] as? String ?? "The update helper could not finish."])}
        guard let result else{throw NSError(domain:"RetrieverUpdater",code:3,userInfo:[NSLocalizedDescriptionKey:"The update helper returned no result."])};return result
    }
    func check(manual:Bool) async {
        guard !busy,manual || automatic else{return};busy=true;message="Checking GitHub for updatesвЂ¦";defer{busy=false}
        do{var args=["check","--current",current,"--platform","macos-arm64","--cache",folder.appendingPathComponent("releases.json").path];if preview{args.append("--preview")};if manual{args.append("--force")};let result=try await run(args);available=result["update"] as? [String:Any];message=available.map{"Retriever \($0["version"] ?? "") is available."} ?? (result["message"] as? String ?? "YouвЂ™re up to date.");if manual{skipped="";laterUntil=0;save()}}
        catch{message=error.localizedDescription}
    }
    func download() async {
        guard !busy,let id=available?["release_id"] as? NSNumber else{return};busy=true;downloading=true;defer{busy=false;downloading=false}
        do{let target=folder.appendingPathComponent("downloads/"+UUID().uuidString,isDirectory:true);var args=["download","--current",current,"--platform","macos-arm64","--release-id",id.stringValue,"--output",target.path];if preview{args.append("--preview")};let result=try await run(args);guard let download=result["download"] as? [String:Any] else{throw CocoaError(.fileReadCorruptFile)};downloadVersion=download["version"] as? String ?? "";receipt=target.appendingPathComponent("download.json").path;save();message="Update downloaded and verified. Open it when youвЂ™re ready."}catch{message=error.localizedDescription}
    }
    func installer() async throws -> URL {guard !busy,downloaded,let receipt else{throw CocoaError(.fileNoSuchFile)};let result=try await run(["verify","--receipt",receipt]);guard let path=result["path"] as? String else{throw CocoaError(.fileReadCorruptFile)};return URL(fileURLWithPath:path)}
}

struct UpdateBadge:View {
    @ObservedObject var updates:AppUpdates
    var show:()->Void
    var body:some View {if updates.notify{Button("Update available",action:show).buttonStyle(.borderless).font(.system(size:10))}}
}
struct UpdatesView:View {
    @ObservedObject var updates:AppUpdates
    var install:(URL) async throws -> Void
    @State private var installing=false
    var body:some View {
        VStack(alignment:.leading,spacing:14){
            Text("Retriever updates").font(.title2)
            Text("Installed: "+updates.current).foregroundStyle(.secondary)
            Text(updates.message).textSelection(.enabled)
            Toggle("Check for updates automatically",isOn:$updates.automatic)
            Toggle("Include preview releases",isOn:$updates.preview).disabled(updates.busy).onChange(of:updates.preview){_,_ in Task{await updates.check(manual:true)}}
            Text("Update checks contact GitHub without sending your library, queries or credentials. Your app data and caches stay in place.").font(.caption).foregroundStyle(.secondary)
            HStack{Button("Check now"){Task{await updates.check(manual:true)}}.disabled(updates.busy)
                if updates.available != nil && !updates.downloaded{Button("Download update"){Task{await updates.download()}}.disabled(updates.busy)}
                if updates.downloading{Button("Cancel download"){updates.cancel()}}
                if updates.downloaded{Button("Open update"){guard !installing else{return};installing=true;Task{defer{installing=false};do{let installer=try await updates.installer();try await install(installer)}catch{updates.message=error.localizedDescription}}}.disabled(updates.busy || installing)}
            }
            if updates.downloaded{Text("Finish active file operations and pause indexing. Retriever will checkpoint its local engine and quit. In the opened disk image, drag Retriever to your Applications folder and replace the previous app, then reopen it.").font(.caption)}
            if updates.available != nil{HStack{Button("Remind me tomorrow"){updates.later()};Button("Skip this version"){updates.skip()}}
                ScrollView{Text(updates.available?["notes"] as? String ?? "").textSelection(.enabled).frame(maxWidth:.infinity,alignment:.leading)}.frame(maxHeight:170)
            }
            if let url=updates.releases{Link("Release notes on GitHub",destination:url)}
        }.padding(24).frame(width:580).disabled(installing)
    }
}

extension WorkspaceModel {
    func openUpdate(_ installer:URL) async throws {
        if !clientMode || usesSharedLibrary {
            let localToken=try String(contentsOf:state.appendingPathComponent("api-token"),encoding:.utf8).trimmingCharacters(in:.whitespacesAndNewlines)
            let endpoint=NativeProfile.endpoint
            var statusRequest=URLRequest(url:URL(string:endpoint+"/api/status")!);statusRequest.timeoutInterval=20;statusRequest.setValue("Bearer "+localToken,forHTTPHeaderField:"Authorization")
            let(data,_)=try await CoordinatorSession.shared.session.data(for:statusRequest);let status=try JSONSerialization.jsonObject(with:data) as? [String:Any] ?? [:]
            guard status["update_protocol"] as? Int==1 else{throw NSError(domain:"RetrieverUpdater",code:4,userInfo:[NSLocalizedDescriptionKey:"This older engine needs a manual restart for its first update. Finish active work before installing."])}
            let updateToken=try String(contentsOf:state.appendingPathComponent("update-token"),encoding:.utf8).trimmingCharacters(in:.whitespacesAndNewlines)
            var request=URLRequest(url:URL(string:endpoint+"/api/runtime/shutdown")!);request.httpMethod="POST";request.timeoutInterval=20;request.setValue("Bearer "+localToken,forHTTPHeaderField:"Authorization");request.setValue(updateToken,forHTTPHeaderField:"X-Retriever-Update-Token");request.setValue("application/json",forHTTPHeaderField:"Content-Type");request.httpBody=try JSONSerialization.data(withJSONObject:["expected_version":status["version"] ?? ""])
            let(reply,response)=try await CoordinatorSession.shared.session.data(for:request);let body=try JSONSerialization.jsonObject(with:reply) as? [String:Any] ?? [:]
            guard (response as? HTTPURLResponse)?.statusCode==200 else{throw NSError(domain:"RetrieverUpdater",code:5,userInfo:[NSLocalizedDescriptionKey:body["error"] as? String ?? "The engine could not prepare for the update."])}
            var stopped=false
            statusRequest.timeoutInterval=1
            let deadline=Date().addingTimeInterval(15)
            while Date()<deadline{try await Task.sleep(for:.milliseconds(100));do{_ = try await CoordinatorSession.shared.session.data(for:statusRequest)}catch let error as URLError where error.code == .cannotConnectToHost {stopped=true;break}catch let error as URLError where error.code == .timedOut {continue}}
            guard stopped else{throw NSError(domain:"RetrieverUpdater",code:6,userInfo:[NSLocalizedDescriptionKey:"The local engine is still stopping. Wait before installing."])}
        }
        guard NSWorkspace.shared.open(installer) else{await start();throw NSError(domain:"RetrieverUpdater",code:7,userInfo:[NSLocalizedDescriptionKey:"macOS could not open the update. The current app remains available."])}
        rememberTabs();NSApp.terminate(nil)
    }
}
