import Foundation
import Testing
@testable import HuraKit

struct AdminTests {
    @Test func writesEverySettingEvenWhenItIsTheDefault() throws {
        let data = try JSONEncoder().encode(Op(op: "set-settings", body: ServerSettings(branchPrefix: "tobias", providers: [])))
        let v = try JSONDecoder().decode(JSONValue.self, from: data)
        #expect(v == .object([
            "op": .string("set-settings"), "branch_prefix": .string("tobias"), "base": .null, "policy": .null,
            "providers": .array([]), "auto_update": .null, "interface": .null,
        ]))
    }

    @Test func forgetsASecretWithANullValue() throws {
        struct Body: Encodable {
            var name: String
            var value: String?
            func encode(to encoder: Encoder) throws {
                var c = encoder.container(keyedBy: CodingKeys.self)
                try c.encode(name, forKey: .name)
                try c.encode(value, forKey: .value)
            }
            enum CodingKeys: String, CodingKey { case name, value }
        }
        let v = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(Body(name: "T", value: nil)))
        #expect(v["value"] == .null)
    }

    @Test func readsIntegrations() throws {
        let raw = """
        {"mcp":[{"name":"jira","url":"http://h:1/mcp","managed":true,"image":"img","state":"crashing","container":"c",
          "secrets":[{"name":"JIRA_TOKEN","set":false,"used_by":["jira"]}],"problem":"JIRA_TOKEN is not set","log":"boom"}],
         "secrets":[{"name":"JIRA_TOKEN","set":false,"used_by":["jira"]}],
         "skills":[{"name":"ship-pr","origin":"/x","uploaded_at":5}],"configured_skills":["review"]}
        """
        let i = try JSONDecoder().decode(Integrations.self, from: Data(raw.utf8))
        #expect(i.mcp.first?.secrets.first?.usedBy == ["jira"])
        #expect(i.skills.first?.uploadedAt == 5)
        #expect(i.configuredSkills == ["review"])
    }

    @Test func readsSettings() throws {
        let raw = #"{"path":"/c.toml","present":false,"settings":{"branch_prefix":null,"base":null,"policy":"open","providers":null,"auto_update":false,"interface":"chat"},"default_branch_prefix":"hura"}"#
        let s = try JSONDecoder().decode(SettingsView.self, from: Data(raw.utf8))
        #expect(s.settings.autoUpdate == false && s.settings.policy == "open" && s.defaultBranchPrefix == "hura")
    }
}
