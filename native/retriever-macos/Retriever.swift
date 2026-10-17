import SwiftUI
import AppKit
import UniformTypeIdentifiers
import Carbon
import Security

final class CoordinatorSession: NSObject, URLSessionTaskDelegate {
    static let shared = CoordinatorSession()
    lazy var session: URLSession = {
        let configuration=URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest=125
        configuration.urlCache=nil
        return URLSession(configuration:configuration,delegate:self,delegateQueue:nil)
    }()
    func urlSession(_ session:URLSession,task:URLSessionTask,willPerformHTTPRedirection response:HTTPURLResponse,newRequest request:URLRequest,completionHandler:@escaping(URLRequest?)->Void) { completionHandler(nil) }
}

enum PairingStore {
    static let service = "app.retriever.candidate.coordinator"
    static func read() -> String {
        let query:[String:Any] = [kSecClass as String:kSecClassGenericPassword,kSecAttrService as String:service,kSecAttrAccount as String:"pairing",kSecReturnData as String:true,kSecMatchLimit as String:kSecMatchLimitOne]
        var value:CFTypeRef?; guard SecItemCopyMatching(query as CFDictionary,&value)==errSecSuccess,let data=value as? Data else { return "" }
        return String(data:data,encoding:.utf8) ?? ""
    }
    static func save(_ token:String) throws {
        let query:[String:Any] = [kSecClass as String:kSecClassGenericPassword,kSecAttrService as String:service,kSecAttrAccount as String:"pairing"]
        let attributes:[String:Any] = [kSecValueData as String:Data(token.utf8),kSecAttrAccessible as String:kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]
        var result=SecItemUpdate(query as CFDictionary,attributes as CFDictionary)
        if result==errSecItemNotFound { result=SecItemAdd(query.merging(attributes){_,new in new} as CFDictionary,nil) }
        guard result==errSecSuccess else { throw NSError(domain:NSOSStatusErrorDomain,code:Int(result)) }
    }
}

struct SourceDevice: Identifiable, Decodable {
    var id: String; var label: String; var roots: [String]; var enabled: Bool
}
struct FileItem: Identifiable {
    let raw: [String: Any]
    var id: String { "\(device):\(path)" }
    var name: String { raw["name"] as? String ?? "" }
    var path: String { raw["path"] as? String ?? "" }
    var parentPath:String {
        guard let separator=path.lastIndex(where:{$0=="/" || $0=="\\"}) else{return path}
        let parent=String(path[..<separator])
        return parent.isEmpty ? String(path[separator]):parent.last==":" ? parent+String(path[separator]):parent
    }
    var device: String { raw["device"] as? String ?? "" }
    var kind: String { raw["kind"] as? String ?? "file" }
    var type: String { raw["type"] as? String ?? "" }
    var modified: String { guard let seconds=(raw["modified"] as? NSNumber)?.doubleValue,seconds>0 else{return "—"};return Date(timeIntervalSince1970:seconds).formatted(date:.numeric,time:.shortened) }
    var typeLabel: String { if kind=="folder" {return "File folder"};return UTType(filenameExtension:URL(fileURLWithPath:path).pathExtension)?.localizedDescription ?? "File" }
    var detail: String { (raw["generated"] as? Bool == true ? "AI description · " : "") + (raw["snippet"] as? String ?? raw["source_status"] as? String ?? "") }
    var size: String { ByteCountFormatter.string(fromByteCount: (raw["size"] as? NSNumber)?.int64Value ?? 0, countStyle: .file) }
    var icon: NSImage { kind == "folder" ? NSWorkspace.shared.icon(for: .folder) : NSWorkspace.shared.icon(for: UTType(filenameExtension: URL(fileURLWithPath: path).pathExtension) ?? .data) }
}

@MainActor final class WorkspaceModel: ObservableObject {
    let updates:AppUpdates
    @Published var showingUpdates=false
    @Published var tabs:[ExplorerTab]=[ExplorerTab()]
    @Published var activeTabID=""
    @Published var visibleColumns:Set<String>=["modified","type","size","device"]
    @Published var closedTabs:[ExplorerTab]=[]
    var tabsReady=false
    var restoringTab=false
    @Published var propertyDefinitions:[[String:Any]]=[]
    @Published var files: [FileItem] = []
    @Published var showingExplorerSetup=false
    @Published var crawlSetupPlan:[String:Any]=[:]
    @Published var assistant:[String:Any]=[:]
    @Published var burnerData:[String:Any]=[:]
    @Published var burnerChecksum=""
    let folderTree=SidebarFolderState()
    @Published var locations:[String:[[String:Any]]]=[:]
    @Published var sumLengths=true
    @Published var searchMode="hybrid"
    @Published var devices: [SourceDevice] = []
    @Published var selection = Set<String>()
    @Published var query = ""
    @Published var quickActive=true
    @Published var message = "Connecting to Retriever…"
    @Published var title = "All files"
    @Published var section = "files"
    @Published var endpoint = "http://127.0.0.1:18780"
    @Published var token = ""
    @Published var clientMode = false
    @Published var previewText = ""
    @Published var previewTitle = ""
    @Published var showingPreview = false
    @Published var configuration: [String: Any] = [:]
    @Published var revision = ""
    @Published var history: [[String: Any]] = []
    @Published var syncPairs: [[String: Any]] = []
    @Published var crawlFolders: [[String: Any]] = []
    @Published var crawlRunning = false
    @Published var crawlProgress = ""
    @Published var diskData: [String:Any] = [:]
    @Published var diskSelection:Int64?
    @Published var healthData:[String:Any]=[:]
    @Published var healthDetail:[String:Any]=[:]
    var healthRequest=0
    #if RETRIEVER_TESTING
    var healthTestProvider:HealthFixtures?
    var capacityTestFixture:[String:Any]?
    #endif
    var diskRequest=0
    var copiedFiles:[FileItem]=[]
    @Published var showingPaste=false
    @Published var busy = false
    @Published var gridMode = false
    @Published var iconSize: Double = 48
    @Published var sortKey = "name"
    @Published var descending = false
    @Published var offset = 0
    @Published var total = 0
    @Published var nextOffset: Int?
    @Published var editingFile: FileItem?
    @Published var metadata: [String:Any] = [:]
    @Published var showingInspector = false
    @Published var pendingPlan: [String:Any] = [:]
    @Published var showingPlan = false
    @Published var backCount = 0
    @Published var forwardCount = 0
    @Published var savedViews: [[String:Any]] = []
    var backStack: [(String, [String:Any], String, String)] = []
    var forwardStack: [(String, [String:Any], String, String)] = []
    var typing: Task<Void, Never>?
    var sequence = 0
    var location: [String: Any] = [:]
    var browseRoute = "library"
    private(set) var localDevice = ""
    private var service: Process?
    let state: URL
    let engine: URL
    let quickSearch: Bool
    let expectedVersion: String
    private let saveConnectionPreferences:Bool
    private let coordinatorEndpoint:String
    init(state: URL? = nil, endpoint: String? = nil, engine: URL? = nil, expectedVersion: String? = nil, quickSearch: Bool = false, usePreferences: Bool = true) {
        let stateLocation=state ?? NativeProfile.state
        self.state = stateLocation
        self.updates=AppUpdates(state:stateLocation)
        self.engine = engine ?? Bundle.main.bundleURL.appendingPathComponent("Contents/Resources/engine/retriever")
        self.expectedVersion = expectedVersion ?? Bundle.main.object(forInfoDictionaryKey:"RetrieverEngineVersion") as? String ?? ""
        self.quickSearch = quickSearch
        self.saveConnectionPreferences=usePreferences && !NativeProfile.isolated
        self.coordinatorEndpoint=endpoint ?? NativeProfile.endpoint
        self.endpoint=self.coordinatorEndpoint
        if usePreferences && !NativeProfile.isolated {clientMode=UserDefaults.standard.bool(forKey:"clientMode");if clientMode {self.endpoint=UserDefaults.standard.string(forKey:"coordinatorEndpoint") ?? self.endpoint;token=PairingStore.read()}}
        if usePreferences && !NativeProfile.isolated && UserDefaults.standard.object(forKey:"clientMode")==nil,let shared=NativeProfile.sharedEndpoint {
            clientMode=true;self.endpoint=shared;token=NativeProfile.sharedToken
        }
    }
    func requireCompatible(_ status:[String:Any]) throws {
        guard status["runtime"] as? String == "rust", !expectedVersion.isEmpty,
              status["version"] as? String == expectedVersion || status["desktop_api_version"] as? Int == 1 || status["version"] as? String == "0.13.0-alpha.20-native" else {
            throw NSError(domain:"Retriever",code:4,userInfo:[NSLocalizedDescriptionKey:"This coordinator uses a different desktop protocol. Update the coordinator or use a compatible Retriever client."])
        }
    }
    var selected: FileItem? { files.first { selection.contains($0.id) } }
    var usesSharedLibrary:Bool {clientMode && !NativeProfile.isolated && endpoint==NativeProfile.sharedEndpoint}
    var canUseLocalFiles:Bool {!clientMode || (usesSharedLibrary && !localDevice.isEmpty)}
    var canOpenSelected:Bool {canUseLocalFiles && selected?.device==localDevice && selected?.path.hasPrefix("/")==true}
    var revealInWorkspace:((FileItem)->Void)?
    @Published var revealRevision=0
    @discardableResult func revealLoadedFile(_ path:String)->Bool {guard let file=files.first(where:{$0.path==path}) else{return false};selection=[file.id];revealRevision &+= 1;return true}
    func revealInRetriever(_ file:FileItem) async {
        await browse(device:file.device,path:file.kind=="folder" ? file.path:file.parentPath)
        if file.kind=="folder" || revealLoadedFile(file.path){return}
        let captured=sequence
        var at:Int?=offset==0 ? nextOffset:0
        await run {
            while let pageOffset=at {
                guard captured==self.sequence else{return}
                self.message="Locating \(file.name)…"
                var body=self.location;body["offset"]=pageOffset;body["sort"]=self.sortKey;body["direction"]=self.descending ? "desc":"asc"
                let reply=try await self.request("explorer/browse",body:body)
                guard captured==self.sequence else{return}
                if (reply["entries"] as? [[String:Any]] ?? []).contains(where:{$0["path"] as? String==file.path}) {
                    self.offset=pageOffset;try await self.loadFiles()
                    if captured==self.sequence && !self.revealLoadedFile(file.path){self.message="The file changed while its folder was loading."}
                    return
                }
                let next=reply["next_offset"] as? Int
                if let next,next<=pageOffset{throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"The folder returned an invalid page."])}
                at=next
            }
            if captured==self.sequence{self.message="The file is no longer present in this folder."}
        }
    }
    func revealSelected() {guard let file=selected else{return};if let revealInWorkspace{revealInWorkspace(file)}else{Task{await revealInRetriever(file)}}}
    func suspendQuickSearch() {typing?.cancel();sequence+=1}

    func request(_ route: String, body: [String: Any]? = nil, revision: String? = nil) async throws -> [String: Any] {
        #if RETRIEVER_TESTING
        if let fixture=capacityTestFixture,route=="disk" || route.hasPrefix("disk/"){guard route=="disk/view" else{throw SmokeError.failed("Synthetic capacity previews cannot scan folders")};return fixture}
        if route=="health" || route.hasPrefix("health/") {guard let fixture=healthTestProvider else{throw SmokeError.failed("Health fixtures must be injected before any test request")};return try fixture.request(route,body:body)}
        #endif
        // Local disk utilities keep using this Mac when search is shared.
        let localUtility=usesSharedLibrary && (route=="disk" || route.hasPrefix("disk/") || route=="burner" || route.hasPrefix("burner/"))
        let auth=localUtility ? try String(contentsOf:state.appendingPathComponent("api-token"),encoding:.utf8).trimmingCharacters(in:.whitespacesAndNewlines):token
        return try await sendRequest(route,body:body,revision:revision,endpoint:localUtility ? coordinatorEndpoint:endpoint,token:auth)
    }
    private func sendRequest(_ route:String,body:[String:Any]?=nil,revision:String?=nil,endpoint:String,token:String) async throws -> [String:Any] {
        guard let base = URL(string: endpoint),base.user==nil,base.password==nil,base.query==nil,base.fragment==nil, base.scheme == "https" || (base.scheme == "http" && ["127.0.0.1", "localhost", "::1"].contains(base.host ?? "")), let url = URL(string: endpoint + "/api/" + route) else { throw NSError(domain: "Retriever", code: 1, userInfo: [NSLocalizedDescriptionKey: "Use HTTPS or a loopback tunnel without credentials in the URL."]) }
        var request = URLRequest(url: url); request.timeoutInterval = 125
        request.setValue("Bearer " + token, forHTTPHeaderField: "Authorization")
        if let body { request.httpMethod = "POST"; request.httpBody = try JSONSerialization.data(withJSONObject: body); request.setValue("application/json", forHTTPHeaderField: "Content-Type") }
        if let revision { request.setValue(revision, forHTTPHeaderField: "If-Match") }
        let (data, response) = try await CoordinatorSession.shared.session.data(for: request)
        guard let value = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw NSError(domain: "Retriever", code: 2, userInfo: [NSLocalizedDescriptionKey: "Invalid coordinator response"]) }
        if let response = response as? HTTPURLResponse, !(200..<300).contains(response.statusCode) { throw NSError(domain: "Retriever", code: response.statusCode, userInfo: [NSLocalizedDescriptionKey: value["error"] as? String ?? "Coordinator request failed"]) }
        return value
    }
    func run(_ operation: () async throws -> Void) async { do { try await operation() } catch is CancellationError {} catch { message = error.localizedDescription } }
    func start() async {await run {try await self.startConnection()}}
    private func startConnection() async throws {
        if let error=NativeProfile.startupError {throw NSError(domain:"Retriever",code:7,userInfo:[NSLocalizedDescriptionKey:error])}
        // Client-only profiles still own local tab and sidebar preferences.
        try FileManager.default.createDirectory(at: self.state, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        if !self.clientMode || self.usesSharedLibrary {
            guard !FileManager.default.fileExists(atPath:self.state.appendingPathComponent("index.sqlite").path) else {throw NSError(domain:"Retriever",code:6,userInfo:[NSLocalizedDescriptionKey:"Choose separate candidate state; the installed index is protected."])}
            let engine = self.engine
            let tokenFile = self.state.appendingPathComponent("api-token")
            if !FileManager.default.fileExists(atPath: tokenFile.path) {
                let process = Process(); process.executableURL = engine; process.arguments = ["--state", self.state.path, "init"]; process.standardOutput = FileHandle.nullDevice; process.standardError = FileHandle.nullDevice; try process.run()
                while process.isRunning { try await Task.sleep(for: .milliseconds(50)) }
                guard process.terminationStatus == 0 else { throw NSError(domain: "Retriever", code: 3, userInfo: [NSLocalizedDescriptionKey: "Candidate state initialization failed"]) }
            }
            let localToken = try String(contentsOf: tokenFile, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
            if !self.clientMode {self.token=localToken}
            do { try self.requireCompatible(try await self.sendRequest("status",endpoint:coordinatorEndpoint,token:localToken)) }
            catch let error as URLError where error.code == .cannotConnectToHost {
                let port=URL(string:coordinatorEndpoint)?.port ?? 18780
                let process = Process(); process.executableURL = engine; process.arguments = ["--state", self.state.path, "serve", "--port", String(port)]; process.standardOutput = FileHandle.nullDevice; process.standardError = FileHandle.nullDevice; try process.run(); self.service = process
                var ready=false
                for _ in 0..<1200 { try await Task.sleep(for:.milliseconds(100)); if !process.isRunning { break };if let status=try? await self.sendRequest("status",endpoint:coordinatorEndpoint,token:localToken) {try self.requireCompatible(status);ready=true;break} }
                guard ready else { throw NSError(domain:"Retriever",code:5,userInfo:[NSLocalizedDescriptionKey:"Rust coordinator did not become ready. Check candidate state and port 18780."]) }
            }
        }
        try self.requireCompatible(try await self.request("status"))
        try await self.refresh()
        if self.quickSearch {
            if !self.quickActive {self.suspendQuickSearch();return}
            if self.query.trimmingCharacters(in:.whitespacesAndNewlines).isEmpty {self.clearQuickSearch()} else {self.queryChanged()}
        } else {await self.restoreTabs()
            #if !RETRIEVER_TESTING
            if self.configuration["configured"] as? Bool != true && !FileManager.default.fileExists(atPath:self.state.appendingPathComponent("onboarding.json").path){self.showingExplorerSetup=true}
            #endif
        }
    }
    func changeConnection(client:Bool,endpoint:String,token:String) async throws {
        let previous=(self.clientMode,self.endpoint,self.token)
        do {
            self.clientMode=client;self.endpoint=client ? endpoint.trimmingCharacters(in:CharacterSet(charactersIn:"/")):coordinatorEndpoint;self.token=token.trimmingCharacters(in:.whitespacesAndNewlines)
            try await startConnection()
            if saveConnectionPreferences {if client {try PairingStore.save(self.token);UserDefaults.standard.set(self.endpoint,forKey:"coordinatorEndpoint")};UserDefaults.standard.set(client,forKey:"clientMode")}
        } catch {self.clientMode=previous.0;self.endpoint=previous.1;self.token=previous.2;try? await refresh();throw error}
    }
    func refresh() async throws {
        let status = try await request("status"); localDevice = status["local_device"] as? String ?? ""
        let data = try await request("config"); configuration = data["config"] as? [String: Any] ?? [:]; revision = data["revision"] as? String ?? ""
        if let raw = configuration["devices"] { devices = try JSONDecoder().decode([SourceDevice].self, from: JSONSerialization.data(withJSONObject: raw)).filter(\.enabled) }
        if clientMode {localDevice=usesSharedLibrary && devices.contains(where:{$0.id==NativeProfile.sharedLocalDevice}) ? NativeProfile.sharedLocalDevice ?? "":""}
        let workspace = try await request("workspace"); history = workspace["history"] as? [[String: Any]] ?? []; syncPairs = workspace["sync_pairs"] as? [[String: Any]] ?? [];savedViews=workspace["views"] as? [[String:Any]] ?? []
    }
    func refreshCrawl() async throws {
        let result=try await request("crawl")
        crawlFolders=result["folders"] as? [[String:Any]] ?? []
        crawlRunning=result["running"] as? Bool ?? false
        let progress=result["progress"] as? [String:Any] ?? [:]
        crawlProgress=crawlRunning ? "Working · \(progress["phase"] as? String ?? "starting"). Search stays available." : "Schedules run while the coordinator is open. Overdue checks are combined."
    }
    func saveCrawl(_ folder:[String:Any],minutes:Int,analysis:String) async -> Bool {
        do {
            _ = try await self.request("crawl/settings",body:["device":folder["device"] ?? "","root":folder["root"] ?? "","revision":folder["revision"] ?? 0,"interval_minutes":minutes,"analysis":analysis])
            try await self.refreshCrawl();return true
        } catch {message=error.localizedDescription;return false}
    }
    func crawlNow(_ folder:[String:Any]) async {
        await run { _ = try await self.request("crawl/run",body:["device":folder["device"] ?? "","root":folder["root"] ?? ""]);try await self.refreshCrawl() }
    }
    func cancelCrawl() async { await run { _ = try await self.request("jobs/cancel",body:[:]);try await self.refreshCrawl() } }
    func loadFiles() async throws {
        if propertyDefinitions.isEmpty{try await reloadPropertyDefinitions()}
        if browseRoute=="assistant" {try await refreshAssistant();return}
        if browseRoute=="burner" {try await refreshBurner();return}
        if browseRoute=="health" {try await loadHealth();return}
        if browseRoute=="disk" {try await loadDisk();return}
        let captured=sequence
        var body=location;body["offset"]=offset;body["sort"]=sortKey;body["direction"]=descending ? "desc" : "asc"
        let reply = try await request(browseRoute=="browse" && location["filesystem"] as? Bool==true ? "explorer/browse":"workspace/" + browseRoute, body: body)
        guard captured==sequence else { return }
        files = (reply["entries"] as? [[String: Any]] ?? []).map(FileItem.init); selection = selection.intersection(files.map(\.id))
        total=reply["total"] as? Int ?? files.count;nextOffset=reply["next_offset"] as? Int
        message = "\(reply["total"] ?? files.count) items · \((reply["cached"] as? Bool == true) ? "Indexed catalogue" : "Live folder")"
        rememberTabs();if visibleColumns.contains("duration"){Task{await readLengths()}}
    }
    func navigate(_ name: String) async { typing?.cancel();sequence+=1;section = name;offset=0;if name != "search" {query=""}; await run {
        switch name {
        case "files", "favorites": self.browseRoute = "library"; self.location = name == "favorites" ? ["filters": ["match": "all", "rules": [["field": "starred", "operator": "is", "value": true]]]] : [:]; self.title = name == "favorites" ? "Favorites" : "All files"; try await self.loadFiles()
        case "activity", "sync", "settings": try await self.refresh()
        case "crawl": try await self.refreshCrawl()
        default: break
        }
    } }
    func browse(device: String, path: String) async {
        if await reuseDestination(folderTabState(device,path)) {return}
        forwardStack=[];forwardCount=0
        if location["path"] as? String != path || location["device"] as? String != device {backStack.append((browseRoute,location,title,query));if backStack.count>100 {backStack.removeFirst()};backCount=backStack.count}
        typing?.cancel();sequence+=1;section = "files";query="";offset=0; browseRoute = "browse";if sortKey=="priority" {sortKey="name"}; location = ["device": device, "path": path,"filesystem":true]; title = path; await run { try await self.loadFiles() }
    }
    func back() async { guard let previous=backStack.popLast() else {return};forwardStack.append((browseRoute,location,title,query));forwardCount=forwardStack.count;backCount=backStack.count;typing?.cancel();sequence+=1;query="";section="files";offset=0;(browseRoute,location,title,query)=previous;if query.isEmpty {await run {try await self.loadFiles()}} else {queryChanged()} }
    func forward() async {guard let next=forwardStack.popLast() else{return};backStack.append((browseRoute,location,title,query));backCount=backStack.count;forwardCount=forwardStack.count;typing?.cancel();sequence+=1;query="";section="files";offset=0;(browseRoute,location,title,query)=next;if query.isEmpty {await run {try await self.loadFiles()}} else {queryChanged()} }
    func filterFiles(_ extensionName:String) async {typing?.cancel();sequence+=1;query="";section="files";title="All files";browseRoute="library";offset=0;location=extensionName.isEmpty ? [:] : ["filters":["match":"all","rules":[["field":"extension","operator":"is","value":extensionName]]]];await run {try await self.loadFiles()} }
    func openView(_ view:[String:Any]) async {let body=view["body"] as? [String:Any] ?? [:];if view["kind"] as? String=="share" {await browse(device:body["device"] as? String ?? "",path:body["path"] as? String ?? "");return};if body["route"] as? String=="browse",let folder=body["location"] as? [String:Any] {await browse(device:folder["device"] as? String ?? "",path:folder["path"] as? String ?? "");return};typing?.cancel();sequence+=1;section="files";title=view["name"] as? String ?? "Saved view";browseRoute="library";offset=0;location=["filters":body["filters"] ?? ["match":"all","rules":[]]];query=body["query"] as? String ?? "";if query.isEmpty {await run {try await self.loadFiles()}}else {queryChanged()} }
    func saveView(_ name:String) async {if !query.trimmingCharacters(in:.whitespacesAndNewlines).isEmpty {message="Pin this tab to keep your search. Clear search and apply filters to save a collection.";return};guard !name.trimmingCharacters(in:.whitespacesAndNewlines).isEmpty else{return};await run {_ = try await self.request("workspace/view",body:["name":name,"kind":"view","body":["query":self.query,"route":self.browseRoute,"location":self.browseRoute=="browse" ? self.location:[:],"filters":self.location["filters"] ?? ["match":"all","rules":[]]]]);try await self.refresh()} }
    func deviceName(_ id:String)->String {devices.first(where:{$0.id==id})?.label ?? id}
    var parentFolder:String? {
        guard browseRoute=="browse",let original=location["path"] as? String else{return nil}
        let path=original.replacingOccurrences(of:"\\",with:"/");let trimmed=String(path.drop(while:{$0==" "})).replacingOccurrences(of:"/+$",with:"",options:.regularExpression)
        guard !trimmed.isEmpty,!(trimmed.count==2 && trimmed.last==":"),let end=trimmed.lastIndex(of:"/") else{return nil}
        let parent=String(trimmed[..<end]);return parent.isEmpty ? "/":parent.last==":" ? parent+"/":parent
    }
    func up() async {if let parent=parentFolder,let device=location["device"] as? String {await browse(device:device,path:parent)} }
    func page(_ offset:Int) async {self.offset=max(0,offset);selection=[];await run {try await self.loadFiles()} }
    func resort() async {await page(0)}
    func select(_ file:FileItem) {if NSEvent.modifierFlags.contains(.command) {if selection.contains(file.id) {selection.remove(file.id)} else {selection.insert(file.id)}} else {selection=[file.id]} }
    func submitSearch() async {typing?.cancel();sequence+=1;let captured=query;let revision=sequence;await run {_ = try await self.search(full:true,captured:captured,revision:revision)}}
    func queryChanged() { if restoringTab {return};typing?.cancel(); sequence += 1; let revision = sequence; let captured = query
        if quickSearch {files=[];selection=[];total=0}
        if captured.trimmingCharacters(in:.whitespacesAndNewlines).isEmpty {
            if quickSearch {clearQuickSearch();return}
            if section=="search" {section="files";typing=Task {await run {try await self.loadFiles()} }}
            return
        }
        section="search"
        typing = Task { do { try await Task.sleep(for: .milliseconds(40)); let complete=try await search(full: false, captured: captured, revision: revision); if !complete {try await Task.sleep(for: .milliseconds(420)); _ = try await search(full: true, captured: captured, revision: revision)} } catch is CancellationError {} catch { if revision == sequence { message = error.localizedDescription } } }
    }
    func clearQuickSearch() {typing?.cancel();sequence+=1;section="search";files=[];selection=[];total=0;offset=0;nextOffset=nil;message="Type to search your files"}
    #if RETRIEVER_TESTING
    var ownedCoordinatorIdentifier: Int32? {service?.processIdentifier}
    func stopTestCoordinator() {typing?.cancel();if let service,service.isRunning {service.terminate();service.waitUntilExit()}}
    #endif
    @discardableResult func search(full: Bool, captured: String, revision: Int) async throws -> Bool {
        guard !captured.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {if quickSearch {clearQuickSearch()} else {try await loadFiles()};return true}
        if ["disk","health","assistant","burner"].contains(browseRoute) {browseRoute="library";location=[:]}
        let encoded = captured.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed.subtracting(CharacterSet(charactersIn: "+&=?#"))) ?? ""
        let reply = try await request("search?q=\(encoded)&mode=\(searchMode)&phase=\(full ? "full" : "instant")")
        guard revision == sequence && captured == query else { return true }
        section = "search"; title = "Search results"; files = (reply["results"] as? [[String: Any]] ?? []).map(FileItem.init)
        selection=selection.intersection(files.map(\.id));total=files.count;offset=0;nextOffset=nil
        if quickSearch && selection.isEmpty,let first=files.first {selection=[first.id]}
        message = "\(files.count) matches · \(String(format:"%.1f",(reply["elapsed_ms"] as? NSNumber)?.doubleValue ?? 0)) ms"
        rememberTabs()
        return (reply["routing"] as? [String:Any])?["complete"] as? Bool == true
    }
    func preview() async { guard let file = selected else { return }; if file.kind == "folder" { await browse(device: file.device, path: file.path); return }; await run {
        let reply = try await self.request(file.raw["indexed_scope"] as? Bool==false ? "explorer/preview":"workspace/preview", body: ["device": file.device, "path": file.path]); self.previewText = reply["text"] as? String ?? "No text preview available"; self.previewTitle = file.name; self.showingPreview = true
    } }
    func open() async { guard let file = selected else { return }; if file.kind == "folder" && !quickSearch { await browse(device: file.device, path: file.path); return }; guard canOpenSelected else { message = "Open this file on its source device. Remote download is pending candidate parity."; return }; if !NSWorkspace.shared.open(URL(fileURLWithPath: file.path)) {message="macOS could not open this file."} }
    func index() async { await run { _ = try await self.request("sync", body: [:]); self.message = "Indexing started. Word search remains available." } }
    func saveSettings() async { await run { let reply = try await self.request("config", body: self.configuration, revision: self.revision); self.revision = reply["revision"] as? String ?? ""; self.message = "Settings saved. Start indexing when ready." } }
    func value(_ role: String, _ key: String) -> String { ((configuration["search"] as? [String: Any])?[role] as? [String: Any])?[key] as? String ?? "" }
    func setValue(_ role: String, _ key: String, _ value: String) { var search = configuration["search"] as? [String: Any] ?? [:]; var settings = search[role] as? [String: Any] ?? [:]; settings[key] = value; search[role] = settings; configuration["search"] = search }
    func probe(_ role: String) async { await run { let settings = (self.configuration["search"] as? [String: Any])?[role] ?? [:]; _ = try await self.request("model-probe", body: ["purpose": role, "settings": settings]); self.message = "\(role.capitalized) local model verified" } }
    func inspect() async {guard let file=selected else{return};await run {let reply=try await self.request("workspace/inspect",body:["scope":"filesystem","device":file.device,"path":file.path]);self.propertyDefinitions=try await self.request("workspace/properties")["properties"] as? [[String:Any]] ?? [];self.editingFile=file;self.metadata=reply;self.showingInspector=true} }
    func addComment(_ body:String) async throws {guard let file=editingFile else{return};metadata=try await request("workspace/comment",body:["scope":"filesystem","device":file.device,"path":file.path,"author":NSUserName(),"body":body])}
    func reviewRename(_ name:String) async {guard let file=editingFile else{return};await run {
        self.pendingPlan=try await self.request("workspace/plan",body:["device":file.device,"scope":"filesystem","operation":"rename","items":[["path":file.path,"name":name]]]);self.showingInspector=false;self.showingPlan=true
    } }
    func applyPlan() async {await run {
        let receipt=try await self.request("workspace/apply",body:["id":self.pendingPlan["id"] ?? ""]);self.showingPlan=false;self.message=receipt["state"] as? String ?? "Inspect the operation receipt";try await self.refresh();try await self.loadFiles()
    } }
    func reviewUndo(_ id:String) async {await run {self.pendingPlan=try await self.request("workspace/undo",body:["id":id]);self.showingPlan=true} }
    func includeFolder() async {
        guard canUseLocalFiles else {message="Pair this Mac with the shared library before including its folders.";return}
        guard let path=await pickPath(title:"Include a folder") else{return}
        await reviewCrawlScope(path)
    }
}

struct CrawlView:View {
    @ObservedObject var model:WorkspaceModel
    var body:some View {
        ScrollView {
            VStack(alignment:.leading,spacing:18) {
                Text("Crawling & learning").font(.largeTitle)
                Text("Build a local catalogue once, then check for changes on your schedule. Unchanged files reuse extracted text, descriptions, and embeddings.").foregroundStyle(.secondary)
                HStack {
                    Button("Refresh status") {Task {await model.run {try await model.refreshCrawl()}}}
                    Button("Cancel current job") {Task {await model.cancelCrawl()}}.disabled(!model.crawlRunning)
                }
                Text(model.crawlProgress).font(.callout).foregroundStyle(.secondary)
                if model.crawlFolders.isEmpty {Text("Include a folder in Settings to choose its crawl schedule.")}
                ForEach(model.crawlFolders.indices,id:\.self) {index in
                    let folder=model.crawlFolders[index]
                    CrawlFolderView(model:model,folder:folder).id("\(folder["device"] ?? ""):\(folder["root"] ?? "")")
                }
            }.padding(24).frame(maxWidth:900,alignment:.leading)
        }.task {
            while !Task.isCancelled {
                do {try await model.refreshCrawl();try await Task.sleep(for:.seconds(5))}
                catch is CancellationError {return}
                catch {model.message=error.localizedDescription;do {try await Task.sleep(for:.seconds(5))} catch {return}}
            }
        }
    }
}
struct CrawlFolderView:View {
    @ObservedObject var model:WorkspaceModel
    let folder:[String:Any]
    @State private var interval=360
    @State private var custom=60
    @State private var analysis="off"
    @State private var dirty=false
    @State private var loadedRevision=0
    private let presets=[0,15,60,360,1440,10080]
    private var minutes:Int {interval == -1 ? custom : interval}
    private var valid:Bool {minutes==0 || (15...43200).contains(minutes)}
    private func when(_ value:Any?) -> String {
        guard let seconds=value as? Double,seconds>0 else {return "Not yet"}
        return Date(timeIntervalSince1970:seconds).formatted(date:.abbreviated,time:.shortened)
    }
    private func load() {
        let saved=folder["interval_minutes"] as? Int ?? 360
        interval=presets.contains(saved) ? saved : -1;custom=max(15,saved);analysis=folder["analysis"] as? String ?? "off";loadedRevision=folder["revision"] as? Int ?? 0;dirty=false
    }
    var body:some View {
        GroupBox {
            VStack(alignment:.leading,spacing:14) {
                Text(folder["root"] as? String ?? "Folder").font(.headline).textSelection(.enabled)
                Text(folder["label"] as? String ?? "").font(.caption).foregroundStyle(.secondary)
                ViewThatFits(in:.horizontal) {
                    HStack(alignment:.top,spacing:20) {schedule;learning}
                    VStack(alignment:.leading,spacing:14) {schedule;learning}
                }
                if interval == -1 {TextField("Minutes (15–43200)",value:$custom,format:.number).onChange(of:custom){_,_ in dirty=true}}
                Text("EmbeddingGemma indexes cached passages and Laya checks relevance locally. Original files remain in their folders.").font(.caption).foregroundStyle(.secondary)
                let report=folder["report"] as? [String:Any] ?? [:]
                let next=(folder["interval_minutes"] as? Int == 0) ? "Manual" : ((folder["next_run"] as? Double ?? 0)>0 ? when(folder["next_run"]) : "Due")
                Text("\(folder["state"] as? String ?? "new") · Last completed: \(when(folder["last_completed"])) · Next: \(next)").font(.caption).foregroundStyle(.secondary)
                Text("\(report["changed"] as? Int ?? 0) changed · \(report["unchanged"] as? Int ?? 0) reused").font(.caption).foregroundStyle(.secondary)
                if let errors=report["errors"] as? [[String:Any]],let first=errors.first {Text("\(errors.count) issue(s): \(first["reason"] as? String ?? "Check source access")").font(.caption).foregroundStyle(.orange)}
                HStack {
                    Button("Save folder settings") {var draft=folder;draft["revision"]=loadedRevision;Task {if await model.saveCrawl(draft,minutes:minutes,analysis:analysis) {dirty=false;loadedRevision += 1}}}.disabled(!dirty || !valid || model.busy)
                    Button("Crawl now") {Task {await model.crawlNow(folder)}}.disabled(model.crawlRunning || model.busy)
                }
            }.padding(12).frame(maxWidth:.infinity,alignment:.leading)
        }.onAppear {load()}.onChange(of:folder["revision"] as? Int ?? 0) {_,_ in if model.busy || !dirty {load()}}
    }
    private var schedule:some View {
        Picker("Check for changes",selection:$interval) {Text("Manual").tag(0);Text("Every 15 minutes").tag(15);Text("Hourly").tag(60);Text("Every 6 hours").tag(360);Text("Daily").tag(1440);Text("Weekly").tag(10080);Text("Custom…").tag(-1)}.onChange(of:interval){_,_ in dirty=true}
    }
    private var learning:some View {
        Picker("Learn from files",selection:$analysis) {Text("Native retrieval · EmbeddingGemma + Laya").tag("off")}.onChange(of:analysis){_,_ in dirty=true}
    }
}

struct ActivityView:View {
    @ObservedObject var model:WorkspaceModel
    var body:some View {
        List(model.history.indices,id:\.self) {index in
            let item=model.history[index]
            let operation=item["operation"] as? String ?? "Operation"
            let state=item["state"] as? String ?? ""
            HStack {
                VStack(alignment:.leading) {Text(operation);Text(state).foregroundStyle(.secondary)}
                Spacer()
                if state=="applied" && ["rename","move","trash"].contains(operation) {
                    Button("Review undo") {Task {await model.reviewUndo(item["id"] as? String ?? "")}}
                }
            }
        }
    }
}

struct PlanReviewView:View {
    @ObservedObject var model:WorkspaceModel
    var items:[[String:Any]] {model.pendingPlan["items"] as? [[String:Any]] ?? []}
    var body:some View {
        VStack(alignment:.leading,spacing:18) {
            Text("Review file operation").font(.title2)
            Text(model.pendingPlan["operation"] as? String ?? "Operation").foregroundStyle(.secondary)
            ScrollView {
                VStack(alignment:.leading,spacing:12) {
                    ForEach(items.indices,id:\.self) {index in
                        let source=items[index]["source"] as? String ?? ""
                        let destination=items[index]["destination"] as? String ?? ""
                        VStack(alignment:.leading,spacing:4) {Text(source);Text("→ " + destination).foregroundStyle(.secondary)}
                    }
                }
            }
            HStack {
                Button("Cancel") {model.showingPlan=false}.keyboardShortcut(.cancelAction)
                Spacer()
                Button("Apply") {Task {await model.applyPlan()}}.buttonStyle(.borderedProminent)
            }
        }.padding(24).frame(width:620,height:360)
    }
}

struct SettingsView: View {
    @ObservedObject var model: WorkspaceModel
    var body: some View {
        Form {
            Section("Updates") {Button("Retriever updates…"){model.showingUpdates=true}}
            Section("Connection") { CoordinatorConnectionForm(model:model) }
            Section("Included folders") {ForEach(model.devices) {device in ForEach(device.roots,id:\.self) {path in Text(path).textSelection(.enabled)}};Button("Include folder on this Mac…") {Task {await model.includeFolder()}}.disabled(!model.canUseLocalFiles)}
            Section("Local search engine") {
                Text(model.state.path).font(.caption).textSelection(.enabled)
                Text("EmbeddingGemma retrieves passages by meaning. Laya checks search relevance. Both run inside Retriever; models, cached passages and metadata stay in the application data folder.")
                ForEach(["embedding", "relevance"], id: \.self) { role in
                    Text(model.value(role, "model"))
                    Button("Check local \(role)") { Task { await model.probe(role) } }
                }
            }
            Button("Save settings") { Task { await model.saveSettings() } }.buttonStyle(.borderedProminent)
        }.formStyle(.grouped)
    }
}
#if !RETRIEVER_TESTING && !RETRIEVER_MODEL_CHECK
@main struct RetrieverApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var delegate
    @StateObject private var model = WorkspaceModel()
    var body: some Scene {
        WindowGroup("Retriever") { WorkspaceView(model: model).frame(minWidth: 800, minHeight: 500).background(MainWindowRegistration(delegate:delegate)).task { delegate.model = model; model.updates.start();await model.start();delegate.prepareQuick() } }.defaultSize(width: 1280, height: 820)
        .commands { CommandGroup(after:.appInfo) {Button("Check for Updates…"){model.showingUpdates=true;Task{await model.updates.check(manual:true)}}}
            CommandGroup(after: .newItem) {
            Button("New tab"){Task {await model.newTab()}}.keyboardShortcut("t")
            Button("Reopen closed tab"){Task {await model.reopenTab()}}.keyboardShortcut("t",modifiers:[.command,.shift])
            Button("Next tab"){Task {await model.cycleTab(1)}}.keyboardShortcut(.tab,modifiers:[.control])
            Button("Previous tab"){Task {await model.cycleTab(-1)}}.keyboardShortcut(.tab,modifiers:[.control,.shift])
            Button("Close tab"){Task {await model.closeTab(model.activeTabID)}}.keyboardShortcut("w")
            Button("Quick Search") { delegate.toggleQuick() }.keyboardShortcut(.space, modifiers: [.control, .option])
        } }
        Settings { SettingsView(model: model).frame(width: 720, height: 700) }
    }
}
#endif
