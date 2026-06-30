%global _topdir %(pwd)/scripts/rpm

Name:           opencode
Version:        %{version}
Release:        1%{?dist}
Summary:        AI coding agent — terminal UI + CLI

License:        MIT
URL:            https://github.com/uitstalie/opencode
Source0:        opencode-linux-x64.tar.gz

BuildArch:      x86_64
AutoReqProv:    no

%description
OpenCode is an AI-powered coding agent with a terminal UI.
Supports multiple LLM providers (OpenAI-compatible API), file
editing, shell commands, web search, and more.

This RPM ships with uitstalie's curated configuration template
(provider endpoints, permission rules, agent prompts, skills).
API keys are NOT included — set them after install.

%prep
%setup -q -c

%build
# Pre-built Bun standalone binary — nothing to compile

%install
rm -rf %{buildroot}

# Binary → /usr/bin/opencode
mkdir -p %{buildroot}%{_bindir}
install -m 755 opencode-linux-x64/opencode %{buildroot}%{_bindir}/opencode

# Config & data → /usr/share/opencode/
mkdir -p %{buildroot}%{_datadir}/opencode
install -m 644 opencode-linux-x64/opencode.json %{buildroot}%{_datadir}/opencode/opencode.json
cp -a opencode-linux-x64/rules %{buildroot}%{_datadir}/opencode/rules 2>/dev/null || true
cp -a opencode-linux-x64/skills %{buildroot}%{_datadir}/opencode/skills 2>/dev/null || true
cp -a opencode-linux-x64/shared-rules %{buildroot}%{_datadir}/opencode/shared-rules 2>/dev/null || true

%files
%{_bindir}/opencode
%{_datadir}/opencode/

%post
echo ""
echo "=== OpenCode %{version}-%{release} installed ==="
echo ""
echo "Quick setup:"
echo "  cp -rn /usr/share/opencode/* ~/.config/opencode/"
echo "  # Then add your API key to ~/.config/opencode/opencode.json"
echo "  #   \"provider\": { \"one_route\": { \"api_key\": \"sk-...\" } }"
echo ""

%preun
if [ $1 -eq 0 ]; then
    echo "Note: ~/.config/opencode/ is preserved. Remove manually if needed."
fi

%changelog
* Thu Jun 26 2025 uitstalie - 0.0.0-1
- Initial package from local config
