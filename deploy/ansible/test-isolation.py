"""Render the deployment templates with example settings, without a Docker host."""
import json
from pathlib import Path
import unittest

from jinja2 import Environment, FileSystemLoader, StrictUndefined
import yaml


ROOT = Path(__file__).resolve().parent


class IsolationTest(unittest.TestCase):
    def setUp(self):
        inventory = yaml.safe_load((ROOT / "inventory/hosts.example.yml").read_text())
        self.settings = inventory["all"]["children"]["presto"]["hosts"]["athena"]
        self.settings.update(presto_image="finance:test", presto_deploy_dir="/opt/finance",
                             presto_domain="finance.example.com")
        self.templates = Environment(loader=FileSystemLoader(ROOT / "templates"),
                                     undefined=StrictUndefined, trim_blocks=True)
        self.templates.filters["to_json"] = json.dumps

    def test_finance_is_only_attached_to_the_external_bridge(self):
        compose = yaml.safe_load(self.templates.get_template("compose.production.yaml.j2")
                                 .render(**self.settings))
        finance = compose["services"]["finance"]
        self.assertNotIn("ports", finance)
        self.assertEqual(compose["networks"], {
            "isolated": {"external": True, "name": self.settings["presto_isolation_network"]},
        })
        self.assertEqual(finance["networks"], {
            "isolated": {"ipv4_address": self.settings["presto_isolation_address"],
                         "aliases": ["presto-finance"]},
        })
        self.assertEqual(finance["extra_hosts"], {
            self.settings["presto_issuer_host"]: self.settings["presto_issuer_proxy_address"],
        })
        self.assertTrue(finance["read_only"])
        self.assertEqual(finance["cap_drop"], ["ALL"])
        self.assertIn("no-new-privileges:true", finance["security_opt"])
        self.assertTrue({"noexec", "nosuid", "nodev"} <= set(finance["tmpfs"][0].split(",")))

    def test_caddy_imports_waf_when_configured(self):
        template = self.templates.get_template("athena.caddy.j2")
        self.assertNotIn("import edge_waf", template.render(**self.settings))
        self.assertIn("import edge_waf", template.render(
            **self.settings, presto_caddy_waf_file="/etc/caddy/00-waf.caddy"))


if __name__ == "__main__":
    unittest.main()
