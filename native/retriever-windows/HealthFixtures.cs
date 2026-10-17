#if RETRIEVER_TESTING
using System.Text.Json.Nodes;
namespace Retriever.Native;

// This provider exists only in test builds. It serves in-memory synthetic JSON
// and cannot enumerate drives, open devices, launch a process, or write a DB.
internal sealed class HealthFixtures {
    readonly JsonObject state;
    public int Requests{get;private set;}
    public HealthFixtures(string path){state=JsonNode.Parse(File.ReadAllText(path))!.AsObject();if(state["fixture_only"]?.GetValue<bool>()!=true||state["probe_available"]?.GetValue<bool>()!=false)throw new InvalidDataException("Expected an offline health fixture");}
    public JsonNode Request(string route,JsonNode? body){
        Requests++;
        if(route=="health/detail")return state["details"]?[body?["id"]?.ToString()??""]?.DeepClone()??throw new InvalidDataException("Unknown fixture drive");
        if(route=="health/settings"){
            if(body?["interval_minutes"]?.GetValue<int>()!=0)throw new InvalidOperationException("Live monitoring is blocked in fixture tests.");
            state["settings"]=body.DeepClone();return body.DeepClone();
        }
        if(route is not ("health" or "health/refresh"))throw new InvalidOperationException("Unexpected fixture route");
        var result=state.DeepClone().AsObject();result.Remove("details");return result;
    }
}
#endif
