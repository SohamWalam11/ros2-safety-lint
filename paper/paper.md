---
title: 'ros2-safety-lint: A Static Verification Linter and Mutation Engine for ROS 2 QoS and SROS2 Security Policies'
tags:
  - Rust
  - ROS 2
  - Robotics
  - Static Analysis
  - Security
authors:
  - name: Jane Doe
    orcid: 0000-0000-0000-0000
    affiliation: 1
affiliations:
 - name: Independent Researcher
   index: 1
date: 26 July 2026
bibliography: paper.bib
---

# Statement of Need

ROS 2 (Robot Operating System 2) is the industry standard for developing robotics applications. However, determining safe combinations of Quality of Service (QoS) parameters and SROS2 security policies is challenging. Configuration incompatibilities are often only discovered at runtime, leading to silent communication failures or security vulnerabilities. `ros2-safety-lint` addresses this by providing offline static validation of ROS 2 configurations, specifically identifying QoS mismatches and insecure SROS2 XML manifests.

# Software Architecture

The linter uses `roxmltree` for zero-allocation XML parsing, which allows it to preserve exact line and column spans for accurate error reporting. It incorporates a pairwise evaluation matrix for QoS compatibility and a rule engine for SROS2 permissions and governance files.

# Research Application

Alongside the linter, this project includes `ros2-safety-mutate`, a mutation testing framework that injects known bugs into seed ROS 2 configurations to empirically evaluate the linter's precision and recall.

# References
