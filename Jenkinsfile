pipeline {
  agent { label 'k8s-agent' }

  parameters {
    choice(name: 'RUN_TYPE', choices: ['ci', 'release', 'stable'], description: 'Lifecycle run type')
    string(name: 'REF_NAME', defaultValue: 'main', description: 'Git ref/branch to build from')
    string(name: 'VERSION', defaultValue: '', description: 'Explicit version override (X.Y.Z)')
    choice(name: 'BUMP', choices: ['patch', 'minor', 'major'], description: 'Version bump when VERSION is empty')
    string(name: 'ACTOR', defaultValue: 'jenkins', description: 'Caller identity')
    booleanParam(name: 'DRY_RUN', defaultValue: true, description: 'Do not publish or mutate remote state')
    booleanParam(name: 'WHEEL_SMOKE', defaultValue: true, description: 'Build wheel as a smoke check')
  }

  options {
    disableConcurrentBuilds()
  }

  environment {
    CARGO_TERM_COLOR = 'always'
  }

  stages {
    stage('checkout') {
      steps {
        checkout scm
        sh '''#!/usr/bin/env bash
set -euo pipefail
if [[ -n "${REF_NAME:-}" ]]; then
  git fetch --all --tags --prune
  git checkout "${REF_NAME}"
fi
'''
      }
    }

    stage('ci') {
      when {
        expression { params.RUN_TYPE == 'ci' || params.RUN_TYPE == 'release' || params.RUN_TYPE == 'stable' }
      }
      steps {
        container('build') {
          sh '''#!/usr/bin/env bash
set -euo pipefail
DEBIAN_FRONTEND=noninteractive apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq python3 python3-venv
make test-rs
make dev
make test-py
if [[ "${WHEEL_SMOKE}" == "true" ]]; then
  cd src/integrations/python
  ../../../.venv/bin/maturin build --release --out ../../../target/wheels
fi
'''
        }
      }
    }

    stage('release') {
      when {
        expression { params.RUN_TYPE == 'release' }
      }
      steps {
        script {
          env.RELEASE_VERSION = resolveVersion('release')
          currentBuild.displayName = "release-${env.RELEASE_VERSION}"
        }
        container('build') {
          sh '''#!/usr/bin/env bash
set -euo pipefail
echo "Preparing release for ${RELEASE_VERSION} (actor=${ACTOR})"
if [[ "${DRY_RUN}" == "true" ]]; then
  echo "DRY_RUN=true: skipping publish/release mutation"
  exit 0
fi
cd src/integrations/python
../../../.venv/bin/maturin build --release --out ../../../target/wheels
'''
        }
      }
    }

    stage('stable') {
      when {
        expression { params.RUN_TYPE == 'stable' }
      }
      steps {
        script {
          env.STABLE_VERSION = resolveVersion('stable')
          currentBuild.displayName = "stable-${env.STABLE_VERSION}"
        }
        container('build') {
          sh '''#!/usr/bin/env bash
set -euo pipefail
if ! git rev-parse --verify --quiet "refs/tags/v${STABLE_VERSION}" >/dev/null; then
  echo "Missing release tag v${STABLE_VERSION}; cannot mark stable"
  exit 1
fi

echo "Marking stable ${STABLE_VERSION} (actor=${ACTOR})"
if [[ "${DRY_RUN}" == "true" ]]; then
  echo "DRY_RUN=true: skipping stable-pointer mutation"
  exit 0
fi

git tag -f "stable-${STABLE_VERSION}" "refs/tags/v${STABLE_VERSION}"
git push origin "refs/tags/stable-${STABLE_VERSION}" --force
'''
        }
      }
    }
  }

  post {
    always {
      archiveArtifacts artifacts: 'target/wheels/*.whl', allowEmptyArchive: true
    }
  }
}

String resolveVersion(String runType) {
  String explicitVersion = params.VERSION?.trim()
  if (explicitVersion) {
    return explicitVersion
  }

  String refName = params.REF_NAME?.trim()
  def releaseMatcher = (refName =~ /^release-(\d+\.\d+\.\d+)$/)
  def stableMatcher = (refName =~ /^stable-(\d+\.\d+\.\d+)$/)

  if (runType == 'release' && releaseMatcher.matches()) {
    return releaseMatcher[0][1]
  }
  if (runType == 'stable' && stableMatcher.matches()) {
    return stableMatcher[0][1]
  }

  if (runType == 'release') {
    return bumpFromTags(params.BUMP ?: 'patch')
  }

  error("Unable to resolve version for ${runType}. Provide VERSION or use ${runType}-X.Y.Z ref")
}

String bumpFromTags(String bumpType) {
  def tagsRaw = sh(script: "git tag --list 'v*'", returnStdout: true).trim()
  if (!tagsRaw) {
    return '0.1.0'
  }

  List<String> versions = tagsRaw
    .split('\n')
    .collect { it.trim() }
    .findAll { it ==~ /^v\d+\.\d+\.\d+$/ }
    .collect { it.substring(1) }

  if (versions.isEmpty()) {
    return '0.1.0'
  }

  List<Integer> latest = versions
    .collect { it.split('\\.').collect { p -> p as Integer } }
    .sort { a, b ->
      (a[0] <=> b[0]) ?: (a[1] <=> b[1]) ?: (a[2] <=> b[2])
    }
    .last()

  int major = latest[0]
  int minor = latest[1]
  int patch = latest[2]

  if (bumpType == 'major') {
    return "${major + 1}.0.0"
  }
  if (bumpType == 'minor') {
    return "${major}.${minor + 1}.0"
  }
  return "${major}.${minor}.${patch + 1}"
}
