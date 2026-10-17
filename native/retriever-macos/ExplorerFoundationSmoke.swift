#if RETRIEVER_TESTING && !RETRIEVER_DESKTOP_TESTING
import AppKit
import SwiftUI
@MainActor final class MemoryFolderHandler:FolderHandlerProvider {
    var current:URL?
    var reject=false
    var ignore=false
    init(_ current:URL){self.current=current}
    func set(_ application:URL) async throws {if reject {throw SmokeError.failed("Declined by fixture")};if !ignore{current=application}}
}
extension NativeSmokeTests {
    @MainActor static func foundation(_ model:WorkspaceModel,_ fixtures:URL,_ output:URL) async throws {
        let menuIcon=RetrieverMenuBarIcon.make()
        try require(menuIcon.isTemplate && menuIcon.size==NSSize(width:18,height:18),"Menu-bar dog uses a native monochrome template")
        try await renderComponent(Image(nsImage:menuIcon).renderingMode(.template).foregroundStyle(.white).padding(18).background(Color.black),output.appendingPathComponent("menu-icon-dark.png"),width:54,height:54)
        try await renderComponent(Image(nsImage:menuIcon).renderingMode(.template).foregroundStyle(.black).padding(18).background(Color.white),output.appendingPathComponent("menu-icon-light.png"),width:54,height:54)
        let child=fixtures.appendingPathComponent("Explorer tree/Nested folder")
        try FileManager.default.createDirectory(at:child,withIntermediateDirectories:true)
        try Data("A safe folder tree fixture".utf8).write(to:child.appendingPathComponent("example.txt"))
        let accepted=try FolderOpenRequest.directories([child,child])
        try require(accepted==[child.standardizedFileURL],"Folder requests deduplicate existing directories")
        for rejected in [[URL(string:"https://example.invalid/folder")!],[child.appendingPathComponent("example.txt")],[child.appendingPathComponent("missing")],Array(repeating:child,count:17)] {
            var blocked=false;do{_ = try FolderOpenRequest.directories(rejected)}catch{blocked=true}
            try require(blocked,"Folder service rejects invalid or oversized selections")
        }
        let service=AppDelegate(),board=NSPasteboard.withUniqueName()
        defer{board.releaseGlobally()}
        board.writeObjects([child as NSURL]);var serviceError:NSString?
        service.openFolders(board,userData:nil,error:&serviceError)
        try require(serviceError==nil && service.pendingFolders.map(\.path)==[child.standardizedFileURL.path],"Finder service queues a folder before the workspace is ready")
        service.openFolders(board,userData:nil,error:&serviceError)
        try require(service.pendingFolders.count==1,"Repeated service requests do not duplicate queued folders")
        board.clearContents();board.writeObjects([child.appendingPathComponent("example.txt") as NSURL])
        service.openFolders(board,userData:nil,error:&serviceError)
        try require(serviceError != nil && service.pendingFolders.count==1,"Finder service reports invalid selections without changing its queue")
        let tree=model.folderTree;tree.restore(model);let key=model.localDevice+":"+fixtures.path;tree.expanded.insert(key);await tree.load(model,model.localDevice,fixtures.path)
        try require(tree.branches[key]?.entries.contains{$0["name"] as? String=="Explorer tree"}==true,"Sidebar expanded the fixture directory")
        tree.save(model);tree.expanded=[];tree.endpoint="";tree.restore(model);try require(tree.expanded.contains(key),"Sidebar expansion survived restore")
        await model.browse(device:model.localDevice,path:child.path);try require(model.files.count==1 && model.files.first?.name=="example.txt","Tree folder navigation works")
        let revision=model.revision;await model.reviewCrawlScope(child.path);try require(model.revision==revision && model.crawlSetupPlan["files_read"] as? Int==0,"Scope review is read-only")
        try await renderComponent(ExplorerSetupPopover(model:model),output.appendingPathComponent("explorer-setup.png"),width:360,height:540);model.showingExplorerSetup=false
        let previous=URL(fileURLWithPath:"/System/Library/CoreServices/Finder.app"),candidate=URL(fileURLWithPath:"/Applications/Retriever Candidate.app")
        let fake=MemoryFolderHandler(previous);let state=model.state.appendingPathComponent("association-fixture");try FileManager.default.createDirectory(at:state,withIntermediateDirectories:true)
        let integration=FolderIntegration(provider:fake,application:candidate,state:state);try await integration.enable();try require(integration.selected,"Folder handler selected in memory");try await integration.restore();try require(fake.current==previous,"Previous folder handler restored in memory")
        try await integration.enable();let newer=URL(fileURLWithPath:"/Applications/Another.app");fake.current=newer;try await integration.restore();try require(fake.current==newer,"Rollback preserves newer folder handler")
        fake.current=previous;fake.reject=true
        do {try await integration.enable();throw SmokeError.failed("Rejected handler was accepted")} catch {try require(fake.current==previous && !FileManager.default.fileExists(atPath:integration.journal.path),"Declined folder request preserves the handler and clears its backup")}
        fake.reject=false;fake.ignore=true
        do {try await integration.enable();throw SmokeError.failed("Ignored handler was accepted")} catch {try require(fake.current==previous && !FileManager.default.fileExists(atPath:integration.journal.path),"Unapplied folder request is not reported as enabled")}
        fake.ignore=false;try await integration.enable();fake.ignore=true
        var restoreFailed=false;do{try await integration.restore()}catch{restoreFailed=true}
        try require(restoreFailed && integration.selected && FileManager.default.fileExists(atPath:integration.journal.path),"Unapplied restore retains the rollback record")
        fake.ignore=false;try await integration.restore()
        try require(fake.current==previous && !FileManager.default.fileExists(atPath:integration.journal.path),"A later restore succeeds and clears the rollback record")
        try await renderComponent(FolderIntegrationPopover(model:model),output.appendingPathComponent("folder-integration.png"),width:360,height:440)
        await model.navigate("files");try await render(model,output.appendingPathComponent("explorer-tree.png"))
    }
}
#endif
