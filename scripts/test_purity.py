import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location("purity", pathlib.Path(__file__).with_name("check-purity.py"))
purity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(purity)


class PurityTests(unittest.TestCase):
    def metadata(self, forbidden):
        return {
            "packages": [{"id": "a", "name": "adk"}, {"id": "b", "name": "helper"},
                         {"id": "c", "name": forbidden}],
            "resolve": {"nodes": [{"id": "a", "dependencies": ["b"]},
                                  {"id": "b", "dependencies": ["c"]},
                                  {"id": "c", "dependencies": []}]},
        }

    def test_transitive_platform_rejected(self):
        self.assertEqual(purity.violations(self.metadata("adk-platform")),
                         ["adk -> helper -> adk-platform"])

    def test_transitive_kubernetes_rejected(self):
        self.assertTrue(purity.violations(self.metadata("k8s-openapi")))

    def test_pure_allowed(self):
        self.assertEqual(purity.violations(self.metadata("serde")), [])
