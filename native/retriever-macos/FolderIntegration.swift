import SwiftUI
import AppKit
import UniformTypeIdentifiers

enum FolderOpenRequest {
    static func directories(_ urls:[URL]) throws -> [URL] {
        guard !urls.isEmpty,urls.count<=16 else {throw failure("Select between one and 16 folders to open in Retriever.")}
        var seen=Set<String>(),result:[URL]=[]
        for url in urls {
            var directory:ObjCBool=false
            guard url.isFileURL,(url.host ?? "").isEmpty || url.host=="localhost",url.path.count<=4096,
                  FileManager.default.fileExists(atPath:url.path,isDirectory:&directory),directory.boolValue else {
                throw failure("The selection contains an unavailable folder or an item that is not a local folder.")
            }
            let normalized=url.standardizedFileURL
            if seen.insert(normalized.path).inserted {result.append(normalized)}
        }
        return result
    }
    private static func failure(_ message:String)->NSError {NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:message])}
}

@MainActor protocol FolderHandlerProvider {
    var current:URL? {get}
    func set(_ application:URL) async throws
}
@MainActor struct SystemFolderHandler:FolderHandlerProvider {
    var current:URL? {NSWorkspace.shared.urlForApplication(toOpen:UTType.folder)}
    func set(_ application:URL) async throws {try await withCheckedThrowingContinuation{(continuation:CheckedContinuation<Void,Error>) in NSWorkspace.shared.setDefaultApplication(at:application,toOpen:UTType.folder){error in if let error{continuation.resume(throwing:error)}else{continuation.resume()}}}}
}
@MainActor final class FolderIntegration {
    struct Backup:Codable {let application:URL;let previous:URL}
    let provider:any FolderHandlerProvider;let application:URL;let journal:URL
    init(provider:(any FolderHandlerProvider)?=nil,application:URL=Bundle.main.bundleURL,state:URL){self.provider=provider ?? SystemFolderHandler();self.application=application;self.journal=state.appendingPathComponent("folder-handler.json")}
    var selected:Bool {provider.current?.standardizedFileURL==application.standardizedFileURL}
    func enable() async throws {
        if selected{return};guard !FileManager.default.fileExists(atPath:journal.path),let previous=provider.current else{throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"Restore the previous integration before changing it again."])}
        try FileManager.default.createDirectory(at:journal.deletingLastPathComponent(),withIntermediateDirectories:true)
        try JSONEncoder().encode(Backup(application:application,previous:previous)).write(to:journal,options:.atomic)
        do {
            try await provider.set(application)
            guard selected else {throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"macOS did not select Retriever for folders."])}
        } catch {
            // A declined/unsupported request must not leave a backup that blocks
            // the next attempt. Retain it only when a changed handler cannot be restored.
            try await restore()
            throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"This Mac declined the folder-default change. Your previous choice is preserved. In Finder, right-click a folder and choose Services → Open in Retriever.",NSUnderlyingErrorKey:error])
        }
    }
    func restore() async throws {
        guard FileManager.default.fileExists(atPath:journal.path) else{return}
        let data=try Data(contentsOf:journal);guard data.count<32768 else{throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"Folder integration backup is invalid."])}
        let backup=try JSONDecoder().decode(Backup.self,from:data)
        guard backup.application.standardizedFileURL==application.standardizedFileURL else{throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"Restore folder integration from the build that enabled it."])}
        if selected{
            try await provider.set(backup.previous)
            guard provider.current?.standardizedFileURL==backup.previous.standardizedFileURL else {
                throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"macOS has not restored the previous folder handler. The backup is retained; try Restore again before removing Retriever."])
            }
        }
        try FileManager.default.removeItem(at:journal)
    }
}
struct FolderIntegrationPopover:View {
    @ObservedObject var model:WorkspaceModel
    @State private var feedback=""
    @State private var working=false
    private var fixture:Bool {
        #if RETRIEVER_TESTING
        return true
        #else
        return NativeProfile.isolated
        #endif
    }
    var body:some View {VStack(alignment:.leading,spacing:12){
        Text("Open folders in Retriever").font(.headline)
        Label("Finder → Services → Open in Retriever",systemImage:"folder.badge.plus").font(.subheadline)
        Text("Right-click a folder in Finder and choose this service. Install Retriever in Applications to make it available. Existing tabs are reused.").foregroundStyle(.secondary)
        Divider()
        Text("Folder default").font(.subheadline.weight(.semibold))
        Text("Some macOS versions decline a system-wide folder default. The Finder service works independently; Finder continues to manage the desktop and system dialogs.").foregroundStyle(.secondary)
        Button("Use Retriever for folders"){change(false)}.disabled(fixture || !model.canUseLocalFiles || working)
        Button("Restore previous folder behavior"){change(true)}.disabled(fixture || !model.canUseLocalFiles || working)
        Text(fixture ? "Fixture preview · OS associations are never changed.":!model.canUseLocalFiles ? "Pair this Mac to enable local folder actions.":feedback).font(.caption)
        Text("Restore the previous handler here before removing the app. Your selection is backed up before any change.").font(.caption).foregroundStyle(.secondary)
    }.padding(16).frame(width:340).task{feedback=FolderIntegration(state:model.state).selected ? "Retriever is selected for folders.":"Folder behavior is unchanged."}}
    private func change(_ restore:Bool){Task{working=true;defer{working=false};do{let integration=FolderIntegration(state:model.state);if restore{try await integration.restore()}else{try await integration.enable()};feedback=restore ? "Previous settings restored; newer choices are preserved.":"Enabled. Open a folder to test the integration."}catch{feedback=error.localizedDescription}}}
}
