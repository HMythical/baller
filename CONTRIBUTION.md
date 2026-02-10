# Contributions 

Baller is a brand new CLI package-manager that will potentially be able to be installed in Linux **AND** Windows.
We've just barely started, it's completely bare bones right now, and has a TON of potential to be a great piece
of sofware. Similarly, there is **alot** that can go wrong with this kind of software, especially given the goal of dual-os capabilities.
The main inspiration for this project is [Chocolatey](https://github.com/chocolatey/choco/tree/develop),
so this project will follow their guidelines on **pull requests**, **code contributions**, **documentation**, and most importantly....**TESTING**

Because this project is very new, potentially dangerous, and has insane potential to one of the greatest package managers for windows, besides chocolatey.
I will explicitly be a pain in the butt when it comes to exactly how much testing you do AND how you write committ messages (I'm not kidding).
A great example of great docuentation on a Pull Request can be found [here](https://github.com/chocolatey/choco/pull/3011).

In addition to that, I ask that you upload pictures/videos/visuals/wtv of your changes running with **NO UNINTENDED EXECUTION BEHAVIOR**... **WHATSOEVER**
Whatever contributuons that you decide to add to baller, you need to 110% sure that it's secure,safe, and easy to understand for other contributors.

Ultimately, all I ask is that you document everything. Code, Tests, Runtime, etc. Anything you want to add, write understandable documentation for it.



## What are you here for?

Baller (or Binary Allocation & Library Launch Environment in Rust if you want to be a nerd about it...), is going to have many components in its codebase,
two seperate operating-system environments, and so much more. You need to know exactly what you are contributing to, and exactly how you're code works when you PR.

### Submitting an Enhancement / Feature Request or Optimizing a function within Baller

If you're looking to improve Baller, then you're in the right place! Welcome! Make sure to familiarize yourself with Chocolatey's practices on PRs and Contributions as baller will implement
several of their practices during development of Baller. Please read the note at the bottom of the section.


### Linux or Windows? Which one?

Baller is a dual-os package-manager. That by-itself is a complex thing to think about. I ask that if you DO contribute to this project. Choose one envrionment and stick t it.
You might have experience in the Linux Kernel, or you might be a certified Windows pro, or you might just be the king of low-level software. That doesnt matter in this project.
The goal is to create a beautiful piece of software. I do not care about how long the development cycle takes. And you shouldn't either. High quality and secure code is the expectation,
and I'd rather you stick to one environment and PR solid code, than write below-decent code but get alot done. 

Essentialy, choose one. If you want to develop Baller in linux, then your contributions, even the ones aftr your initial contribution, should stay in Linux. Same for windows. Security > Speed.

If your PR has changes for both environments, your contribution may not be accepted. So please stick to one operating system environment! Trust me, you will hate it, but I'm adding this rule for a reason.

### **PLEASE NOTE: YOU NEED TO LOG A GITHUB ISSUE IF YOU ARE SUBMITTING AN ENHANCEMENT**...
Its because there are less constraints, rather than reporting an issue. 



### Pre-requisites

- Fork the repo
- Sign the Contributor License Agreement (CLA) - I'm not trying to get in legal trouble. I checked the CLA, it should not have predatory legal language in any way, shape, or form.
I will not accept any contributions without it.
- Sign it for each Baller project that required it. Simple.
- Why am I having you sign this? Julien Ponge. Specifically section 5.1 of his blog post. Reference his post [here](https://julien.ponge.org/blog/in-defense-of-contributor-license-agreements/).
- [Sign the CLA Here](https://cla-assistant.io/HMythical/baller)

#### Do I really have to sign the CLA?
Yes. End of discussion.




# Sooo...How do I... you know...contribute? 
## Choose an environment

As previously stated before, If you want to contribute to baller, you will need to contribute to ONE operating-system environment. Windows OR Linux, one or the other. The only exception to this rule will be for the OS-detection service. That will be just rust with libraries and stuff. 



### Rust
Rust will be the **main** language used for this project. Rust was chosen because of it's memory saftey compared to C and C++ as well as its compatibility with both Linux and Windows (I chose rust primarily because of this reason). However, rust isnt just the only language we will have to use to make this package manager work!

### Powershell
Windows is weird and doesnt use bash normally. We all know about cmd and PowerShell... All contributitions that relate to powershell, rust or something else, must be able to work with Powershell v3 (v2 we can get to eventually). 

### C?? C#?? .NET?? What about those?
Honestly, I am not opposed to using those languages, however, I do not know exactly how C and Rust would interact if we used them bot. Same thing with C# and the .NET Framework. I am not opposed to contributions with those languages, but you the contributor as well as I need to know how they will interact under the hood. If you do use these languages, make sure to document,compare, and summarize how different the assembly instructions are and make sure the instructions that come out of compilation do not cause memory errors.

Additionally... **please write memory safe code**... I'm going to leave it at that....


### Should I use other languages like Go or Ruby or something else?

No. I only allow languages I understand under the hood, or I can easily learn how they work under the hood.

## Code Formatting / Design

Until contributions start to pile in in **different** languages, the rust files will have configuration files to enforce certain formatting standards.

All naming conventions will be listed here for youre reference. It will also be in the Baller Docs (when I get around to creating them):

- Non-OS specific Structs must all be PascalCase, with context included. [Context][Purpose][TypeSuffix]
- All variables must have explicit declaration
- Unless you require OS-specific behavior, use Rust primitives DIRECTLY
- Non-OS specific variables must be in snake_case and explicitly declared
- Operating System specific variables must be prefixed with "os_"
- Collections and Tuples must be plural
- Booleans must be prefixed with is_,has_,can_,should_ and their grammatical opposites when dealing with false Boolean values (isnt_,hasnt_,cant_,etc)
- OS-specific Structs must be prefixed with their respective OS. (E.g LinuxPackageInfo)
- Configuration structs must be suffixed with Config (E.g InstallConfig)
- Builders must be suffixed with Builder (E.g PackageQueryBuilder)
- Data Transfer Objects must be suffixed with DTO
- Errors must be suffixed with their OS
  e.g

  pub enum InstallError{
    Linux(LinuxError),
    Windows(WindowsError),
    Common(CommonError),

  }
- 

## Documenting,Debugging and Testing
### How should I document my code?
### How should I test it?

## So... about my IDE... which one should I use?!

## The Survival Guide for Commits (I'm "stealing" chocolatey's framework for commits... please dont sue me)

### The Pull Request

### Feedback?

## I like how Chocolatey does "x" thing... Can we "implement" it like they do?




# Other important general information

This contributions file is definetly going to change as Baller gains contributrs and development time. This file is meant to lay down the framework for how Baller will want to grow in order to be a reliable piece of software. 



# Conclusion

**Your contributions will never be forgotten! Thank you for putting your time, energy, and you passion into this project. I am eternally grateful for any who decide to help make Baller come to life! You're work will pave the way for this software to grow exponentially!**

