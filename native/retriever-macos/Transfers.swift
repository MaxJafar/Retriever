import SwiftUI

@MainActor extension WorkspaceModel {
    func copyFiles() {
        let selected=files.filter{selection.contains($0.id)}
        guard !selected.isEmpty,selected.allSatisfy({$0.kind=="file"}),Set(selected.map(\.device)).count==1 else{message="Select regular files on one device.";return}
        copiedFiles=selected;message="\(selected.count) files ready to copy · open a destination folder and paste"
    }
    func pasteFiles() {
        guard !copiedFiles.isEmpty else{message="Copy files in Retriever first.";return}
        guard browseRoute=="browse",location["path"] as? String != nil else{message="Open a destination folder before pasting.";return}
        showingPaste=true
    }
    func runCopy(_ plan:[String:Any],progress:([String:Any])->Void) async throws -> [String:Any] {
        guard let id=plan["id"] as? String else{throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"Missing copy plan."])}
        _ = try await request("workspace/start",body:["id":id])
        while true {
            let result=try await request("workspace/operation/"+id)
            if !["preview","running"].contains(result["state"] as? String ?? ""){return result}
            progress(result["transfer"] as? [String:Any] ?? [:]);try await Task.sleep(for:.milliseconds(250))
        }
    }
}
