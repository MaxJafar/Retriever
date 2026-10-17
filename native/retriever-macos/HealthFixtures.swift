#if RETRIEVER_TESTING
import Foundation

// No hardware, networking, process launch, or database writes in this provider.
@MainActor final class HealthFixtures {
    private var state:[String:Any]
    private(set) var requests=0
    init(url:URL) throws {
        state=try JSONSerialization.jsonObject(with:Data(contentsOf:url)) as? [String:Any] ?? [:]
        guard state["fixture_only"] as? Bool==true,state["probe_available"] as? Bool==false else{throw SmokeError.failed("Expected offline health fixtures")}
    }
    func request(_ route:String,body:[String:Any]?) throws->[String:Any] {
        requests+=1
        if route=="health/detail" {guard let id=body?["id"] as? String,let detail=(state["details"] as? [String:[String:Any]])?[id] else{throw SmokeError.failed("Unknown fixture drive")};return detail}
        if route=="health/settings" {guard let body,(body["interval_minutes"] as? NSNumber)?.intValue==0 else{throw SmokeError.failed("Live monitoring is blocked in fixture tests")};state["settings"]=body;return body}
        guard ["health","health/refresh"].contains(route) else{throw SmokeError.failed("Unexpected fixture route")}
        var result=state;result.removeValue(forKey:"details");return result
    }
}
#endif
